use std::{env, path::PathBuf, process::Command};

fn main() {
    let runtime = env::var_os("SCRIPTC_M06_RUNTIME")
        .map(PathBuf::from)
        .expect("SCRIPTC_M06_RUNTIME must point at @scriptc/runtime");
    let scalar = env::var_os("SCRIPTC_M06_SCALAR_C")
        .map(PathBuf::from)
        .expect("SCRIPTC_M06_SCALAR_C must point at generated scalar.lib.c");
    let include =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../toolchains/scriptc/m0/include");
    let shim = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../toolchains/scriptc/m0/shims/scr_lib_cleanup.c");
    let u64_probe = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../toolchains/scriptc/m0/soft-float/u64_add.c");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let clang = env::var_os("SCRIPTC_CLANG").unwrap_or_else(|| "/usr/lib/llvm-20/bin/clang".into());
    let ar =
        env::var_os("SCRIPTC_LLVM_AR").unwrap_or_else(|| "/usr/lib/llvm-20/bin/llvm-ar".into());
    let nm = env::var_os("SCRIPTC_LLVM_NM").unwrap_or_else(|| "/usr/bin/llvm-nm".into());
    let size = env::var_os("SCRIPTC_LLVM_SIZE").unwrap_or_else(|| "/usr/bin/llvm-size".into());
    let sources = [
        scalar,
        u64_probe,
        runtime.join("src/scr_library.c"),
        runtime.join("src/scr_number.c"),
        runtime.join("src/scr_string.c"),
        runtime.join("src/scr_array.c"),
        runtime.join("src/scr_bytes.c"),
        runtime.join("src/scr_cycle.c"),
        runtime.join("src/scr_error.c"),
        runtime.join("src/scr_exception.c"),
        runtime.join("src/scr_object.c"),
        shim,
    ];
    let mut objects = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        println!("cargo:rerun-if-changed={}", source.display());
        let object = out.join(format!("scriptc-m0-{index}.o"));
        let status = Command::new(&clang)
            .args([
                "--target=riscv64-unknown-elf",
                "-march=rv64emac",
                "-mabi=lp64e",
                "-ffreestanding",
                "-fno-builtin",
                "-fPIC",
                "-ffunction-sections",
                "-fdata-sections",
                "-O2",
                "-DSCR_LIB",
                "-I",
                include.to_str().unwrap(),
                "-I",
                runtime.join("src").to_str().unwrap(),
                "-c",
                source.to_str().unwrap(),
                "-o",
                object.to_str().unwrap(),
            ])
            .status()
            .unwrap_or_else(|error| {
                panic!("start ScriptC C compile for {}: {error}", source.display())
            });
        assert!(
            status.success(),
            "ScriptC C compile failed for {}",
            source.display()
        );
        objects.push(object);
    }
    let archive = out.join("libscriptc_m0_runtime.a");
    let mut command = Command::new(&ar);
    command.arg("rcs").arg(&archive).args(&objects);
    let status = command.status().expect("start llvm-ar");
    assert!(status.success(), "llvm-ar failed for {}", archive.display());
    let report_path = env::var_os("SCRIPTC_M06_DEPENDENCY_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| out.join("runtime-dependencies.json"));
    let mut report = String::from("{\n  \"target\": \"rv64emac/lp64e\",\n  \"objects\": [\n");
    for (index, (source, object)) in sources.iter().zip(objects.iter()).enumerate() {
        let undefined = Command::new(&nm)
            .args(["-u", object.to_str().unwrap()])
            .output()
            .expect("start llvm-nm");
        let mut symbols = String::from("[");
        let mut first_symbol = true;
        let mut scriptc = Vec::new();
        let mut libc = Vec::new();
        let mut libm = Vec::new();
        let mut compiler_rt = Vec::new();
        let mut other = Vec::new();
        let undefined_text = String::from_utf8_lossy(&undefined.stdout);
        for line in undefined_text.lines() {
            let Some(symbol) = line.split_whitespace().last() else {
                continue;
            };
            if !first_symbol {
                symbols.push(',');
            }
            symbols.push_str(&format!("\"{}\"", json_escape(symbol)));
            first_symbol = false;
            if symbol.starts_with("scr_") {
                scriptc.push(symbol);
            } else if symbol.starts_with("__") && symbol != "__assert_fail" {
                compiler_rt.push(symbol);
            }
            if [
                "abort",
                "calloc",
                "free",
                "getenv",
                "isinf",
                "isnan",
                "malloc",
                "memcmp",
                "memcpy",
                "memmove",
                "memset",
                "realloc",
                "snprintf",
                "stpcpy",
                "strcmp",
                "strlen",
                "strtol",
                "strtod",
                "trunc",
                "vsnprintf",
            ]
            .contains(&symbol)
            {
                libc.push(symbol);
            } else if [
                "exp2", "fabs", "fmod", "floor", "isfinite", "ldexp", "signbit",
            ]
            .contains(&symbol)
            {
                libm.push(symbol);
            } else if !symbol.starts_with("scr_")
                && !(symbol.starts_with("__") && symbol != "__assert_fail")
            {
                other.push(symbol);
            }
        }
        let size_output = Command::new(&size)
            .arg(object)
            .output()
            .expect("start llvm-size");
        let size_text = String::from_utf8_lossy(&size_output.stdout);
        let size_line = size_text.lines().nth(1).unwrap_or("");
        let columns = size_line.split_whitespace().collect::<Vec<_>>();
        let text = columns.first().copied().unwrap_or("0");
        let data = columns.get(1).copied().unwrap_or("0");
        let bss = columns.get(2).copied().unwrap_or("0");
        let comma = if index + 1 == sources.len() { "" } else { "," };
        report.push_str(&format!(
            "    {{\"source\":\"{}\",\"object\":\"{}\",\"text\":{},\"data\":{},\"bss\":{},\"undefined\":{},\"scriptc\":{},\"libc\":{},\"libm\":{},\"compiler_rt\":{},\"other\":{}}}{}\n",
            json_escape(&source.display().to_string()),
            json_escape(&object.display().to_string()),
            text,
            data,
            bss,
            format!("{}]", symbols),
            string_array(&scriptc),
            string_array(&libc),
            string_array(&libm),
            string_array(&compiler_rt),
            string_array(&other),
            comma,
        ));
    }
    report.push_str("  ]\n}\n");
    std::fs::write(&report_path, report).expect("write M0.6 dependency report");
    println!(
        "cargo:warning=ScriptC M0.6 dependency report: {}",
        report_path.display()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=scriptc_m0_runtime");
    println!("cargo:rerun-if-env-changed=SCRIPTC_M06_RUNTIME");
    println!("cargo:rerun-if-env-changed=SCRIPTC_M06_SCALAR_C");
    println!("cargo:rerun-if-env-changed=SCRIPTC_CLANG");
    println!("cargo:rerun-if-env-changed=SCRIPTC_LLVM_AR");
    println!("cargo:rerun-if-env-changed=SCRIPTC_LLVM_NM");
    println!("cargo:rerun-if-env-changed=SCRIPTC_LLVM_SIZE");
    println!("cargo:rerun-if-env-changed=SCRIPTC_M06_DEPENDENCY_REPORT");
}

fn json_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn string_array(values: &[&str]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| format!("\"{}\"", json_escape(value)))
            .collect::<Vec<_>>()
            .join(",")
    )
}
