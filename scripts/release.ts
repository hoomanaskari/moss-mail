import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { release } from "./lib/release.ts";

try {
  await release(process.argv.slice(2), fileURLToPath(new URL("..", import.meta.url)), {
    env: process.env,
    log: console.log,
    run(command, args, options) {
      const output = execFileSync(command, args, { cwd: options.cwd, env: options.env,
        encoding: "utf8", maxBuffer: 16 * 1024 * 1024,
        stdio: options.capture ? ["inherit", "pipe", "inherit"] : "inherit" });
      return output ?? "";
    },
  });
} catch (error) {
  console.error(`Stopped: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
}
