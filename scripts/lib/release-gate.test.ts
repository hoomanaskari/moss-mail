import { afterEach, describe, expect, it } from "vitest";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { snapshotSource } from "./release-source";
import { validateAndPackage } from "./release-gate";
const folders: string[] = [];
afterEach(() => { for (const dir of folders.splice(0)) rmSync(dir, { recursive: true, force: true }); });
function fixture() {
 const dir = mkdtempSync(join(tmpdir(), "moss-gate-")); folders.push(dir);
 const root = join(dir, "source");
 const manifest = snapshotSource([{path:"main.ts",bytes:Buffer.from("source")}],root);
 return {root,manifest};
}
describe("release validation gate", () => {
 it("awaits an asynchronous check failure before packaging", async () => {
  const {root,manifest}=fixture(); const calls:string[]=[];
  const failure=Promise.reject(new Error("async check failed")); void failure.catch(()=>{});
  await expect(validateAndPackage(root,manifest,phase=>{calls.push(phase); return phase==="check"?failure:Promise.resolve();})).rejects.toThrow("async check failed");
  expect(calls).toEqual(["install","check"]);
 });
 it("checks source after an asynchronous phase finishes", async () => {
  const {root,manifest}=fixture();const calls:string[]=[];
  await expect(validateAndPackage(root,manifest,async phase=>{calls.push(phase);await Promise.resolve();writeFileSync(join(root,"main.ts"),"changed");})).rejects.toThrow("source changed");
  expect(calls).toEqual(["install"]);
 });
 it("validates and packages the same snapshot before allowing publication", async () => {
  const {root,manifest}=fixture(); const calls: string[]=[];
  await validateAndPackage(root,manifest,phase=>{calls.push(phase);});
  expect(calls).toEqual(["install","check","package"]);
 });
 it.each(["install","check","package"] as const)("propagates a failed %s and stops", async failure=>{
  const {root,manifest}=fixture(); const calls:string[]=[];
  await expect(validateAndPackage(root,manifest,phase=>{calls.push(phase);if(phase===failure)throw new Error("command failed");})).rejects.toThrow("command failed");
  expect(calls).toEqual(["install","check","package"].slice(0,["install","check","package"].indexOf(failure)+1));
 });
 it.each(["before","install","check","package"])("rejects source changes during %s", async when=>{
  const {root,manifest}=fixture();const calls:string[]=[];
  if(when==="before")writeFileSync(join(root,"main.ts"),"changed");
  await expect(validateAndPackage(root,manifest,phase=>{calls.push(phase);if(phase===when)writeFileSync(join(root,"main.ts"),"changed");})).rejects.toThrow("source changed");
  expect(calls).toEqual(when==="before"?[]:["install","check","package"].slice(0,["install","check","package"].indexOf(when)+1));
 });
});
