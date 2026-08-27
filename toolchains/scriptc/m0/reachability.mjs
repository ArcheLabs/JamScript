import { execFileSync } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname);
const output = resolve(process.env.SCRIPTC_REACHABILITY_OUT ?? resolve(root, "out-reachability"));
const clang = process.env.SCRIPTC_CLANG ?? "/usr/lib/llvm-20/bin/clang";
const nm = process.env.SCRIPTC_LLVM_NM ?? "/usr/bin/llvm-nm";
await mkdir(output, { recursive: true });
const archive = resolve(root, "out/scalar/scalar.lib.a");
const rootObject = resolve(output, "scalar-root.o");
const map = resolve(output, "scalar-link.map");
const executable = resolve(output, "scalar-host-reachability");
execFileSync(clang, ["-O0", "-ffunction-sections", "-fdata-sections", "-c", resolve(root, "reachability/scalar_root.c"), "-o", rootObject]);
execFileSync(clang, ["-no-pie", rootObject, archive, `-Wl,--gc-sections,--unresolved-symbols=ignore-all,-Map=${map}`, "-o", executable]);
const mapText = await readFile(map, "utf8");
const reachable = [...mapText.matchAll(/^(\S*scalar\.lib\.a\(([^)]+)\))$/gm)]
  .map((match) => match[2])
  .filter((name, index, names) => names.indexOf(name) === index)
  .join("\n") + "\n";
const unresolved = execFileSync(nm, ["-u", executable], { encoding: "utf8" });
await writeFile(resolve(output, "scalar-reachable-symbols.txt"), reachable);
await writeFile(resolve(output, "scalar-unresolved-symbols.txt"), unresolved);
console.log(JSON.stringify({ map, reachable: reachable.trim().split("\n"), unresolved: unresolved.trim().split("\n") }, null, 2));
