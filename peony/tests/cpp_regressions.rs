//! Regression tests for the C++ link bugs found and fixed while making peony
//! link a real `std::cout << ... ` program. Each test corresponds to a distinct
//! root-cause bug; together they pin the PIE C++ path (copy relocations,
//! `.gnu.hash` indexing, GNU-property merge, zero-size anchor sections) that
//! used to produce a binary that crashed on startup.
//!
//! All link through `cc -B<peony>` as a **PIE** — the default for cc/rustc and
//! the configuration where these bugs live. If the C++ toolchain is unavailable
//! the test skips (returns) rather than failing.

mod common;
use common::*;

/// The headline case: `std::cout << "..."` in a PIE. This crashed with a NULL
/// vtable in `std::ostream::sentry` because the copy-relocated `_ZSt4cout` was
/// not indexed in the executable's `.gnu.hash`/`.hash`, so libstdc++ bound cout
/// to its own uninitialised copy instead of the executable's. Fixed by hashing
/// copy-reloc imports + omitting the empty stub `.gnu.hash`.
#[test]
fn cpp_iostream_cout() {
    let dir = workdir("cpp_cout");
    let src = r#"
        #include <iostream>
        int main() { std::cout << "peony-cout\n"; return 0; }
    "#;
    let Some(exe) = cc_b_pie(&dir, "cout", &[("cout.cpp", src)], true) else {
        eprintln!("skipping: C++ toolchain unavailable");
        return;
    };
    let (rc, out) = run_capture(&exe);
    assert_eq!(rc, 0, "iostream PIE exe must exit 0 (was SIGSEGV=139)");
    assert_eq!(out, "peony-cout\n");
}

/// Full program: STL container + exceptions + iostream together. Exercises
/// `.eh_frame`/`.gcc_except_table`, the merged `.note.gnu.property`, and the
/// `_ZTI*` typeinfo copy-reloc exclusion all at once.
#[test]
fn cpp_vector_exceptions_iostream() {
    let dir = workdir("cpp_full");
    let src = r#"
        #include <iostream>
        #include <vector>
        #include <stdexcept>
        int main() {
            std::vector<int> v{1, 2, 3};
            long s = 0;
            try {
                for (int x : v) s += x;
                if (s != 6) throw std::runtime_error("bad sum");
            } catch (const std::exception& e) {
                std::cerr << e.what();
                return 1;
            }
            std::cout << "sum=" << s << "\n";
            return 0;
        }
    "#;
    let Some(exe) = cc_b_pie(&dir, "full", &[("full.cpp", src)], true) else {
        return;
    };
    let (rc, out) = run_capture(&exe);
    assert_eq!(rc, 0);
    assert_eq!(out, "sum=6\n");
}

/// A throw/catch that never touches iostream — isolates the exception runtime
/// (`__cxa_throw`, `.eh_frame`, typeinfo) from the stream-object path.
#[test]
fn cpp_exceptions_only() {
    let dir = workdir("cpp_exc");
    let src = r#"
        #include <stdexcept>
        int main() {
            try { throw std::runtime_error("x"); }
            catch (const std::exception&) { return 0; }
            return 1;
        }
    "#;
    let Some(exe) = cc_b_pie(&dir, "exc", &[("exc.cpp", src)], true) else {
        return;
    };
    assert_eq!(run(&exe), 0);
}

/// A static constructor/destructor pair — exercises `.init_array`/`.fini_array`
/// and the `register_tm_clones`/`__TMC_END__` zero-size-anchor handling that, if
/// the empty `.tm_clone_table` is dropped, jumps through a wild pointer.
#[test]
fn cpp_static_ctor_dtor() {
    let dir = workdir("cpp_ctor");
    let src = r#"
        #include <cstdio>
        struct S { S(){ std::printf("ctor\n"); } ~S(){ std::printf("dtor\n"); } };
        static S g;
        int main() { std::printf("main\n"); return 0; }
    "#;
    let Some(exe) = cc_b_pie(&dir, "ctor", &[("ctor.cpp", src)], true) else {
        return;
    };
    let (rc, out) = run_capture(&exe);
    assert_eq!(rc, 0);
    assert_eq!(out, "ctor\nmain\ndtor\n", "static ctor/dtor ordering");
}

/// The PIE C++ output must be a structurally valid ELF: a strict checker
/// (`eu-elflint`) used to reject it ("not in executable format" in BFD) because
/// of a bogus `.symtab_shndx` emitted for `SHN_ABS` symbols. Assert no
/// `.symtab_shndx` section is present in a small C++ exe (no link has > 65535
/// sections, so it must never appear).
#[test]
fn cpp_no_spurious_symtab_shndx() {
    let dir = workdir("cpp_shndx");
    let src = r#"
        #include <iostream>
        int main(){ std::cout << "x\n"; return 0; }
    "#;
    let Some(exe) = cc_b_pie(&dir, "shndx", &[("shndx.cpp", src)], true) else {
        return;
    };
    let sections = readelf(&exe, &["-S"]);
    assert!(
        !sections.contains(".symtab_shndx"),
        ".symtab_shndx must not appear in a normal executable (BFD rejects it):\n{sections}"
    );
}

/// The merged `.note.gnu.property` must be a single note, not one per input
/// object. We assert exactly one `NT_GNU_PROPERTY_TYPE_0` note is present.
#[test]
fn cpp_single_gnu_property_note() {
    let dir = workdir("cpp_prop");
    let src = r#"int main(){ return 0; }"#;
    let Some(exe) = cc_b_pie(&dir, "prop", &[("prop.cpp", src)], true) else {
        return;
    };
    let notes = readelf(&exe, &["-n"]);
    let count = notes.matches("NT_GNU_PROPERTY_TYPE_0").count();
    // 0 is acceptable (toolchain may not emit the property); >1 is the bug.
    assert!(
        count <= 1,
        "expected a single merged .note.gnu.property, found {count}:\n{notes}"
    );
}
