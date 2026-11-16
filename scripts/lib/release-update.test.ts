import { afterEach, describe, expect, it } from "vitest";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { verifyUpdate } from "./release-update";
const folders:string[]=[];
afterEach(()=>{for(const dir of folders.splice(0))rmSync(dir,{recursive:true,force:true});});
function fixture(comment="timestamp:1\tfile:app.tar.gz\tversion:1.2.3") {
 const root=mkdtempSync(join(tmpdir(),"moss-update-test-"));folders.push(root);
 const archive=join(root,"app.tar.gz"),key=join(root,"key"),pub=join(root,"pub"),sig=join(root,"sig");
 const minisign=(args:string[])=>execFileSync("minisign",args,{encoding:"utf8",stdio:["ignore","pipe","pipe"]});
 minisign(["-G","-W","-s",key,"-p",pub]);writeFileSync(archive,"disposable archive fixture");
 minisign(["-S","-s",key,"-m",archive,"-x",sig,"-t",comment]);
 const signature=readFileSync(sig).toString("base64"),publicKey=readFileSync(pub).toString("base64");
 const url="https://github.com/hoomanaskari/moss-mail/releases/download/v1.2.3/Moss_1.2.3_aarch64.app.tar.gz";
 const manifest={version:"1.2.3",platforms:{"darwin-aarch64":{url,signature}}};
 return {root,key,pub,sig,archive,signature,publicKey,url,version:"1.2.3",manifest,minisign};
}
describe("update artifact verification",()=>{
 it("verifies the archive, public key, authenticated version and update manifest",()=>{
  const f=fixture();const calls:string[][]=[];
  expect(()=>verifyUpdate(f,args=>{calls.push(args);return f.minisign(args);})).not.toThrow();
  expect(calls).toHaveLength(1);expect(calls[0]).toContain("-V");
  expect(existsSync(dirname(calls[0][calls[0].indexOf("-p")+1]))).toBe(false);
 });
 it("rejects a signature made by a different key",()=>{
  const f=fixture(),other=fixture();expect(()=>verifyUpdate({...f,publicKey:other.publicKey},f.minisign)).toThrow();
 });
 it("rejects modified archive bytes",()=>{
  const f=fixture();writeFileSync(f.archive,"changed");expect(()=>verifyUpdate(f,f.minisign)).toThrow();
 });
 it.each(["timestamp:1\tfile:app.tar.gz", "version:1.2.4", "version:1.2.3\tversion:1.2.4"])("rejects an absent, different or ambiguous signed version: %s",comment=>{
  const f=fixture(comment);expect(()=>verifyUpdate(f,f.minisign)).toThrow("signed version");
 });
 it.each(["version","url","signature","platform","null"])("rejects an inconsistent %s in latest.json",kind=>{
  const f=fixture();let manifest:unknown=f.manifest;
  if(kind==="version")f.manifest.version="9.9.9";
  if(kind==="url")f.manifest.platforms["darwin-aarch64"].url="https://wrong.example.invalid/update";
  if(kind==="signature")f.manifest.platforms["darwin-aarch64"].signature="wrong";
  if(kind==="platform")manifest={version:f.version,platforms:{}};
  if(kind==="null")manifest=null;
  expect(()=>verifyUpdate({...f,manifest},f.minisign)).toThrow("update manifest");
 });
 it.each(["publicKey","signature"] as const)("refuses malformed %s encodings",field=>{
  const f=fixture();const value={...f,[field]:"not base64!"};
  if(field==="signature")value.manifest.platforms["darwin-aarch64"].signature=value.signature;
  expect(()=>verifyUpdate(value,f.minisign)).toThrow("base64");
 });
});
