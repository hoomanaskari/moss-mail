// pnpm set-version 0.2.0: sets Moss's version everywhere it's kept (see lib/version.ts).
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { setVersion } from "./lib/version.ts";

process.exitCode = setVersion(process.argv.slice(2), {
  read: (path) => readFileSync(path, "utf8"),
  write: (path, text) => writeFileSync(path, text),
  run: (command, args) => void execFileSync(command, args, { stdio: "inherit" }),
  log: (line) => console.log(line),
});
