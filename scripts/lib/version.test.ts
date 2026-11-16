import { describe, expect, it } from "vitest";
import { currentVersion, isVersion, setVersion, versionFiles, type Io } from "./version";

const files: Record<string, string> = {
  "package.json": '{\n  "name": "moss-mail",\n  "version": "0.1.0",\n  "dependencies": { "x": { "version": "9.9.9" } }\n}\n',
  "src-tauri/tauri.conf.json": '{\n  "productName": "Moss",\n  "version": "0.1.0",\n  "plugins": {}\n}\n',
  "Cargo.toml": '[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.1.0"\nedition = "2021"\n\n[workspace.dependencies]\nserde = { version = "1" }\n',
};

/** A pretend repo: what was written, run and said. */
function repo(start = files) {
  const disk = { ...start };
  const ran: string[][] = [];
  const said: string[] = [];
  const io: Io = {
    read: (path) => disk[path],
    write: (path, text) => void (disk[path] = text),
    run: (cmd, args) => void ran.push([cmd, ...args]),
    log: (line) => void said.push(line),
  };
  return { disk, ran, said, io };
}

describe("isVersion", () => {
  it.each(["0.2.0", "1.10.3", "1.0.0-beta.1"])("takes %s", (v) => expect(isVersion(v)).toBe(true));
  it.each(["", "v0.2.0", "0.2", "0.2.0.1", "one"])("refuses %j", (v) => expect(isVersion(v)).toBe(false));
});

describe("setVersion", () => {
  it("sets Moss's own version in every file, and only that one", () => {
    const { disk, ran, said, io } = repo();
    expect(setVersion(["0.2.0"], io)).toBe(0);
    expect(disk["package.json"]).toContain('  "version": "0.2.0",');
    expect(disk["package.json"]).toContain('"version": "9.9.9"');
    expect(disk["src-tauri/tauri.conf.json"]).toContain('  "version": "0.2.0",');
    expect(disk["Cargo.toml"]).toContain('[workspace.package]\nversion = "0.2.0"');
    expect(disk["Cargo.toml"]).toContain('serde = { version = "1" }');
    expect(ran).toEqual([["cargo", "update", "--workspace", "--offline"]]);
    expect(said).toEqual(["Moss is now 0.2.0 (was 0.1.0) in package.json, src-tauri/tauri.conf.json, Cargo.toml and Cargo.lock."]);
  });

  it("says how to use it when the version is missing or wrong, and changes nothing", () => {
    for (const args of [[], ["v0.2"]]) {
      const { disk, ran, said, io } = repo();
      expect(setVersion(args, io)).toBe(1);
      expect(disk).toEqual(files);
      expect(ran).toEqual([]);
      expect(said).toEqual(["Give the new version, like: pnpm set-version 0.2.0"]);
    }
  });

  it("stops before writing anything when a file has no version to change", () => {
    const { disk, ran, said, io } = repo({ ...files, "Cargo.toml": "[workspace]\n" });
    expect(setVersion(["0.2.0"], io)).toBe(1);
    expect(disk).toEqual({ ...files, "Cargo.toml": "[workspace]\n" });
    expect(ran).toEqual([]);
    expect(said).toEqual(["Couldn’t find the version in Cargo.toml."]);
  });

  it("knows the files", () => {
    expect(versionFiles.map((f) => f.path)).toEqual(["package.json", "src-tauri/tauri.conf.json", "Cargo.toml"]);
  });
});

it("reads the current version from the app's config", () => {
  expect(currentVersion(repo().io)).toBe("0.1.0");
});
