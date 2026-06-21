use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use peony_layout::Layout;
use peony_object::{InputArena, InputObject};
use peony_prof::TraceField;
use peony_reloc::{ApplyCtx, RelocError};
use peony_symbols::SymbolTable;

use crate::input_work::{
    AcceptedWorkItemRange,
    AcceptedWorkRanges,
    WorkItem,
    WorkSummary,
    collect_input_work_items,
    usize_to_u64,
    validate_work_item_ranges,
};
use crate::{EmitError, Result, SectionWriteFilter};

#[derive(Clone, Copy)]
struct OutputBuffer {
    ptr: usize,
    len: usize,
}

struct DispatchCtx<'a> {
    arena: &'a InputArena,
    output: OutputBuffer,
    reloc: &'a ApplyCtx<'a>,
    output_path: &'a Path,
}

#[derive(Default)]
struct EmitTotals {
    sections: u64,
    bytes: u64,
    relocs: u64,
}

#[derive(Default)]
struct SharedEmitTotals {
    sections: AtomicU64,
    bytes: AtomicU64,
    relocs: AtomicU64,
}

impl SharedEmitTotals {
    fn add(&self, totals: &EmitTotals) {
        self.sections.fetch_add(totals.sections, Ordering::Relaxed);
        self.bytes.fetch_add(totals.bytes, Ordering::Relaxed);
        self.relocs.fetch_add(totals.relocs, Ordering::Relaxed);
    }

    fn snapshot(&self) -> EmitTotals {
        EmitTotals {
            sections: self.sections.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            relocs: self.relocs.load(Ordering::Relaxed),
        }
    }
}

pub(crate) fn copy_input_sections(
    arena: &InputArena,
    objects: &[InputObject],
    symbols: &SymbolTable,
    layout: &Layout,
    filter: SectionWriteFilter<'_>,
    output_path: &Path,
    buf_ptr: usize,
    buf_len: usize,
) -> Result<()> {
    let sym_index = {
        let _t = peony_prof::trace("emit:sym-index-build");
        peony_reloc::SymIndex::build(objects, symbols)
    };
    let reloc_ctx = ApplyCtx {
        symbols,
        layout,
        shared: layout.shared,
        sym_index: Some(&sym_index),
    };
    let work_items = collect_input_work_items(layout, objects, filter);
    let accepted_ranges = validate_work_item_ranges(&work_items, buf_len)?;
    peony_prof::record_items("emit", work_items.len() as u64);
    let _t = WorkSummary::from_items(&work_items).map_or_else(
        || peony_prof::trace("emit:section-copy-dispatch"),
        |summary| peony_prof::trace_fields("emit:section-copy-dispatch", summary.trace_fields()),
    );
    let dispatch = DispatchCtx {
        arena,
        output: OutputBuffer {
            ptr: buf_ptr,
            len: buf_len,
        },
        reloc: &reloc_ctx,
        output_path,
    };
    let totals = dispatch_parallel(&work_items, &accepted_ranges, &dispatch)?;
    record_emit_totals(&totals);
    Ok(())
}

fn dispatch_parallel(
    work_items: &[WorkItem<'_>],
    accepted_ranges: &AcceptedWorkRanges,
    ctx: &DispatchCtx<'_>,
) -> Result<EmitTotals> {
    if work_items.is_empty() {
        return Ok(EmitTotals::default());
    }

    const PARALLEL_THRESHOLD: usize = 2048;
    if work_items.len() < PARALLEL_THRESHOLD {
        peony_prof::event_fields(
            "emit-dispatch",
            [
                TraceField::text("mode", "serial"),
                TraceField::count("work_items", usize_to_u64(work_items.len())),
            ],
        );
        let mut totals = EmitTotals::default();
        for (item_index, &item) in work_items.iter().enumerate() {
            let Some(accepted_range) = accepted_ranges.range_for_item(item_index) else {
                continue;
            };
            process_item(item, accepted_range, ctx, &mut totals)
                .map_err(|e| reloc_error(ctx.output_path, e))?;
        }
        return Ok(totals);
    }

    let num_threads = rayon::current_num_threads().max(1).min(work_items.len());
    peony_prof::event_fields(
        "emit-dispatch",
        [
            TraceField::text("mode", "parallel"),
            TraceField::count("work_items", usize_to_u64(work_items.len())),
            TraceField::count("workers", usize_to_u64(num_threads)),
        ],
    );

    const BATCH_SIZE: usize = 256;
    let batch_count = work_items.len().div_ceil(BATCH_SIZE);
    peony_prof::event_fields(
        "emit-dispatch",
        [
            TraceField::text("mode", "range-batches"),
            TraceField::count("work_items", usize_to_u64(work_items.len())),
            TraceField::count("batches", usize_to_u64(batch_count)),
            TraceField::count("batch_size", usize_to_u64(BATCH_SIZE)),
        ],
    );
    let batches: Vec<(usize, usize)> = (0..batch_count)
        .map(|batch| {
            let start = batch * BATCH_SIZE;
            let end = (start + BATCH_SIZE).min(work_items.len());
            (start, end)
        })
        .collect();
    let error_slot = Mutex::new(None);
    let totals = SharedEmitTotals::default();

    ws_deque::scheduler::run(num_threads, batches, |(start, end), _spawner| {
        let mut batch_totals = EmitTotals::default();
        for (batch_offset, &item) in work_items[start..end].iter().enumerate() {
            let item_index = start + batch_offset;
            let Some(accepted_range) = accepted_ranges.range_for_item(item_index) else {
                continue;
            };
            if let Err(error) = process_item(item, accepted_range, ctx, &mut batch_totals) {
                record_error(&error_slot, error);
                break;
            }
        }
        totals.add(&batch_totals);
    });

    let error = match error_slot.into_inner() {
        Ok(error) => error,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(error) = error {
        return Err(reloc_error(ctx.output_path, error));
    }
    Ok(totals.snapshot())
}

fn record_emit_totals(totals: &EmitTotals) {
    peony_prof::count("sections_emitted", totals.sections);
    peony_prof::count("relocs_applied", totals.relocs);
    peony_prof::record_bytes("emit", totals.bytes);
}

fn reloc_error(output_path: &Path, error: RelocError) -> EmitError {
    tracing::error!(output = %output_path.display(), %error, "relocation error");
    EmitError::Reloc(error)
}

fn record_error(slot: &Mutex<Option<RelocError>>, error: RelocError) {
    let mut guard = match slot.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if guard.is_none() {
        *guard = Some(error);
    }
}

fn process_item(
    item: WorkItem<'_>,
    accepted_range: AcceptedWorkItemRange,
    ctx: &DispatchCtx<'_>,
    totals: &mut EmitTotals,
) -> std::result::Result<(), RelocError> {
    let obj = item.obj;
    let isec = item.isec;

    let data_len = isec.data.len();
    if data_len == 0
        || item
            .file_off
            .checked_add(data_len)
            .is_none_or(|end| end > ctx.output.len || !accepted_range.contains(item.file_off, end))
    {
        return Ok(());
    }

    // SAFETY: layout assigns every input contribution a unique, non-overlapping
    // file range before these work items are produced.
    let sec_buf = unsafe {
        std::slice::from_raw_parts_mut((ctx.output.ptr + item.file_off) as *mut u8, data_len)
    };

    if peony_prof::trace_detail_enabled() {
        peony_prof::detail_event_fields(
            "emit:input-section",
            [
                TraceField::text("object", obj.path.as_str()),
                TraceField::text(
                    "section",
                    String::from_utf8_lossy(isec.name.as_bytes()).into_owned(),
                ),
                TraceField::count("input_shndx", usize_to_u64(item.input_section_index)),
                TraceField::byte_range("file", usize_to_u64(item.file_off), usize_to_u64(data_len)),
                TraceField::addr_range("va", item.section_va, usize_to_u64(data_len)),
                TraceField::count("relocs", usize_to_u64(isec.relocs.len())),
            ],
        );
    }

    sec_buf.copy_from_slice(ctx.arena.bytes(isec.data));
    totals.sections += 1;
    totals.bytes += data_len as u64;

    for reloc in &isec.relocs {
        peony_reloc::apply_reloc(ctx.reloc, obj, item.obj_id, reloc, item.section_va, sec_buf)?;
        totals.relocs += 1;
    }

    Ok(())
}
