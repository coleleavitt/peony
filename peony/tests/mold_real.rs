//! Real C/C++ programs linked by **peony driven via `cc -B`** — the same way
//! mold's own test suite drives the toolchain (`$CC -B. -o exe a.o; run; grep`).
//! These are faithful equivalents of mold's `libc / C runtime` tests, now
//! actually executing through peony. Each `skip`s cleanly if the toolchain is
//! unavailable (so CI without a C compiler still passes).

mod common;
use common::*;

fn run_cc(dir: &str, src: &str, cxx: bool) -> Option<(i32, String)> {
    let d = workdir(dir);
    let exe = cc_b(&d, "exe", &[("a.c", src)], cxx)?;
    Some(run_capture(&exe))
}

/// mold `hello.sh`: printf round-trip.
#[test]
fn hello_world() {
    let Some((rc, out)) = run_cc(
        "m_hello",
        "#include <stdio.h>\nint main(void){ printf(\"Hello world\\n\"); return 0; }\n",
        false,
    ) else {
        return;
    };
    assert_eq!(rc, 0);
    assert!(out.contains("Hello world"), "got: {out:?}");
}

/// Exit status propagation.
#[test]
fn exit_status() {
    let Some((rc, _)) = run_cc("m_exit", "int main(void){ return 42; }\n", false) else {
        return;
    };
    assert_eq!(rc, 42);
}

/// Integer math + libc `printf("%d")`.
#[test]
fn integer_math() {
    let Some((rc, out)) = run_cc(
        "m_math",
        "#include <stdio.h>\nint main(void){ int s=0; for(int i=1;i<=8;i++) s+=i; printf(\"%d\\n\", s); return 0; }\n",
        false,
    ) else {
        return;
    };
    assert_eq!(rc, 0);
    assert_eq!(out.trim(), "36");
}

/// libc string + heap functions.
#[test]
fn string_and_heap() {
    let Some((rc, out)) = run_cc(
        "m_str",
        "#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n\
         int main(void){ char*p=malloc(32); strcpy(p,\"mold\"); strcat(p,\"-peony\"); \
         printf(\"%s %zu\\n\", p, strlen(p)); free(p); return 0; }\n",
        false,
    ) else {
        return;
    };
    assert_eq!(rc, 0);
    assert_eq!(out.trim(), "mold-peony 10");
}

/// Multiple translation units linked together (cross-TU call).
#[test]
fn multiple_translation_units() {
    let d = workdir("m_mtu");
    let Some(exe) = cc_b(
        &d,
        "exe",
        &[
            (
                "a.c",
                "extern int helper(int);\nint main(void){ return helper(40); }\n",
            ),
            ("b.c", "int helper(int x){ return x + 2; }\n"),
        ],
        false,
    ) else {
        return;
    };
    assert_eq!(run(&exe), 42);
}

/// C++ global constructor runs before `main` (`.init_array` / `DT_INIT_ARRAY`).
#[test]
fn cpp_global_ctor() {
    let Some((rc, _)) = run_cc(
        "m_cpp_ctor",
        "#include <cstdio>\nstatic int x;\nstruct S{ S(){ x = 42; } };\n\
         static S s;\nint main(){ return x; }\n", // 42 only if the ctor ran
        true,
    ) else {
        return;
    };
    assert_eq!(rc, 42);
}

/// C++ `new`/`delete` (operator new from libstdc++).
#[test]
fn cpp_new_delete() {
    let Some((rc, out)) = run_cc(
        "m_cpp_new",
        "#include <cstdio>\nint main(){ int*p=new int(42); printf(\"%d\\n\",*p); \
         int r=*p; delete p; return r==42?0:1; }\n",
        true,
    ) else {
        return;
    };
    assert_eq!(rc, 0);
    assert_eq!(out.trim(), "42");
}

/// libc `qsort` with a comparator callback (function pointers through the GOT/PLT).
#[test]
fn qsort_callback() {
    let Some((rc, out)) = run_cc(
        "m_qsort",
        "#include <stdio.h>\n#include <stdlib.h>\n\
         static int cmp(const void*a,const void*b){ return *(const int*)a-*(const int*)b; }\n\
         int main(void){ int v[]={5,2,8,1,9,3}; qsort(v,6,sizeof(int),cmp); \
         for(int i=0;i<6;i++) printf(\"%d\", v[i]); printf(\"\\n\"); return 0; }\n",
        false,
    ) else {
        return;
    };
    assert_eq!(rc, 0);
    assert_eq!(out.trim(), "123589");
}
