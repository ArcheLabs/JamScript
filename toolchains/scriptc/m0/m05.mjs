import { execFileSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname);
const output = resolve(process.env.SCRIPTC_M05_OUT ?? resolve(root, "out-m05"));
const runtime = resolve(process.env.SCRIPTC_M0_RUNTIME ?? resolve(root, "../node_modules/@scriptc/runtime"));
const clang = process.env.SCRIPTC_CLANG ?? "/usr/lib/llvm-20/bin/clang";
const nm = process.env.SCRIPTC_LLVM_NM ?? "/usr/bin/llvm-nm";
const size = process.env.SCRIPTC_LLVM_SIZE ?? "/usr/bin/llvm-size";
const targetArgs = [
  "--target=riscv64-unknown-elf",
  "-march=rv64emac",
  "-mabi=lp64e",
  "-ffreestanding",
  "-fno-builtin",
  "-O2",
  "-DSCR_LIB",
  "-I", resolve(root, "include"),
  "-I", resolve(runtime, "src"),
];
await mkdir(output, { recursive: true });

function run(command, args) {
  try {
    return { ok: true, stdout: execFileSync(command, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }) };
  } catch (error) {
    return { ok: false, stdout: error.stdout?.toString() ?? "", stderr: error.stderr?.toString() ?? String(error), status: error.status ?? null };
  }
}

async function compile(name, source) {
  const object = resolve(output, `${name}.o`);
  const compile = run(clang, [...targetArgs, "-c", source, "-o", object]);
  const symbols = run(nm, ["-u", object]);
  const objectSize = run(size, [object]);
  return { name, source, object, compile, undefinedSymbols: symbols.ok ? symbols.stdout : symbols.stderr, size: objectSize.ok ? objectSize.stdout : objectSize.stderr };
}

const probes = [
  await compile("double-add", resolve(root, "soft-float/double_add.c")),
  await compile("u64-add", resolve(root, "soft-float/u64_add.c")),
  await compile("scriptc-scalar", resolve(root, "out/scalar/scalar.lib.c")),
];
const report = {
  target: "riscv64-unknown-elf / rv64emac / lp64e",
  runtime,
  clang,
  probes,
  note: "Objects are intentionally not linked into a PVM artifact until the reachable deterministic runtime and compiler-rt policy are resolved.",
};
await writeFile(resolve(output, "m05.json"), JSON.stringify(report, null, 2));
console.log(JSON.stringify(report, null, 2));
if (probes.some((probe) => !probe.compile.ok)) process.exitCode = 1;
