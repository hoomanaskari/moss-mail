// Native test harness. Uses synthetic mail and an isolated WebKit data store.
import { createServer } from "node:http";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { writeFileSync } from "node:fs";
import assert from "node:assert/strict";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const temporary = await mkdtemp(join(tmpdir(), "moss-native-email-"));
const fixtures = join(temporary, "fixtures.json");
const executable = join(temporary, "probe");
const run = promisify(execFile);

try {
  await run("pnpm", ["vitest", "run", "src/design-system/organisms/EmailBody/nativeDocuments.test.ts"], {
    cwd: root,
    env: { ...process.env, MOSS_NATIVE_EMAIL_FIXTURES: fixtures },
    timeout: 60000,
  });
  await run("/usr/bin/swiftc", [fileURLToPath(new URL("./Probe.swift", import.meta.url)), "-o", executable], {
    cwd: root,
    timeout: 60000,
  });

  const requests = [];
  const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=", "base64");
  const server = createServer((req, res) => {
    requests.push({ path: req.url, referer: req.headers.referer ?? null });
    res.writeHead(200, { "content-type": "image/png", "cache-control": "no-store" });
    res.end(png);
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const origin = `http://127.0.0.1:${server.address().port}`;
  try {
    const { stdout } = await run(executable, [fixtures, origin], { timeout: 30000 });
    const observations = JSON.parse(stdout.trim());
    if (process.env.MOSS_NATIVE_EMAIL_REPORT) {
      writeFileSync(process.env.MOSS_NATIVE_EMAIL_REPORT, JSON.stringify({ observations, requests }, null, 2), { mode: 0o600 });
    }
    assert.equal(observations.length, 4);
    for (const row of observations) {
      assert.equal(row.compromised, false, row.name + " ran attacker script");
      assert.match(row.body, new RegExp("Native fixture " + row.name));
    }
    for (const name of ["blocked", "csp-blocked"]) {
      assert.deepEqual(requests.filter(r => r.path.startsWith("/" + name + "/")), [], name + " fetched remote content");
    }
    for (const name of ["allowed", "csp-allowed"]) {
      assert.deepEqual(
        requests.filter(r => r.path.startsWith("/" + name + "/")).map(r => r.path).sort(),
        [`/${name}/background.png`, `/${name}/image.png`],
        name + " request set",
      );
      assert(observations.find(r => r.name === name).images.some(i => i.width === 1), name + " positive image control did not render");
    }
    assert.equal(requests.length, 4, "Unexpected resource request outside the allowlist");
    assert(requests.every(r => r.referer === null), "Referrer leaked");
    console.log(JSON.stringify({ nativeCases: observations.length, requests: requests.map(r => r.path), result: "passed" }));
  } finally {
    await new Promise(resolve => server.close(resolve));
  }
} finally {
  await rm(temporary, { recursive: true, force: true });
}
