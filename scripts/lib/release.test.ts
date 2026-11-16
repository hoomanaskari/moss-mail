import { afterEach, describe, expect, it, vi } from "vitest";
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { snapshotSource } from "./release-source";
import { release, type ReleaseIo } from "./release";
vi.mock("node:fs", async importOriginal => {
 const actual = await importOriginal<typeof import("node:fs")>();
 return { ...actual, copyFileSync: vi.fn(actual.copyFileSync), rmSync: vi.fn(actual.rmSync) };
});
vi.mock("./release-source.ts", async importOriginal => {
 const actual = await importOriginal<typeof import("./release-source")>();
 return { ...actual, snapshotSource: vi.fn(actual.snapshotSource) };
});
const folders: string[] = [];
afterEach(() => { vi.mocked(copyFileSync).mockReset(); vi.mocked(rmSync).mockReset(); vi.mocked(snapshotSource).mockReset(); for (const dir of folders.splice(0)) rmSync(dir, { recursive: true, force: true }); });
function fixture() {
 const root=mkdtempSync(join(tmpdir(),"moss-release-test-"));folders.push(root);
 const keys=mkdtempSync(join(tmpdir(),"moss-release-test-keys-"));folders.push(keys);
 execFileSync("minisign",["-G","-W","-s",join(keys,"key"),"-p",join(keys,"pub")],{stdio:"ignore"});
 const publicKey=readFileSync(join(keys,"pub")).toString("base64");
 const git=(...args:string[])=>execFileSync("git",args,{cwd:root,encoding:"utf8"}).trim();
 git("init","-q","-b","main");git("config","user.name","Test");git("config","user.email","test@example.invalid");
 mkdirSync(join(root,"src-tauri"));mkdirSync(join(root,"docs/releases"),{recursive:true});
 writeFileSync(join(root,"src-tauri/tauri.conf.json"),JSON.stringify({version:"1.2.3",plugins:{updater:{pubkey:publicKey}}}));
 writeFileSync(join(root,"docs/releases/1.2.3.md"),"Release notes");writeFileSync(join(root,"main.ts"),"original");
 git("add",".");git("commit","-qm","fixture");const sha=git("rev-parse","HEAD");git("update-ref","refs/remotes/origin/main",sha);
 const logs:string[]=[];
 const calls:{command:string,args:string[],cwd:string,env:NodeJS.ProcessEnv}[]=[];
 let tamperProvenance=""; let architecture="arm64"; let artifactKind="";
 let failure=""; let mutate=""; let remoteTag=""; let githubTags="";
 const io:ReleaseIo={env:{PATH:process.env.PATH,HOME:process.env.HOME,NODE_OPTIONS:"injected",CARGO_TARGET_DIR:"/outside",GIT_DIR:"/outside",GH_TOKEN:"synthetic-test-token",GH_HOST:"wrong.example.invalid"},log:message=>{logs.push(message);},run(command,args,options){
  calls.push({command,args,cwd:options.cwd,env:options.env});
  const phase=command==="pnpm"?args[0]:command==="sh"?"package":"";
  if(phase===tamperProvenance&&tamperProvenance)writeFileSync(join(options.cwd,"target/moss-release-source.json"),"tampered");
  if(phase===failure&&failure)throw new Error("injected command failure");
  if(phase===mutate&&mutate)writeFileSync(join(options.cwd,"main.ts"),"changed");
  if(command==="git") {
   if(args[0]==="fetch")return "";
   if(args[0]==="ls-remote")return remoteTag;
   return execFileSync("git",args,{cwd:options.cwd,env:options.env,encoding:"utf8"});
  }
  if(command==="uname")return architecture;
  if(command==="minisign"&&args[0]==="-V")return execFileSync(command,args,{encoding:"utf8",stdio:["ignore","pipe","pipe"]});
  if(command==="gh")return args[0]==="api"?githubTags:"";
  if(command==="sh") {
   const out=join(options.cwd,"target/release/moss-release/1.2.3");mkdirSync(out,{recursive:true});
   for(const name of ["Moss_1.2.3_aarch64.dmg","Moss_1.2.3_aarch64.app.tar.gz","Moss_1.2.3_aarch64.app.tar.gz.sig","latest.json"])writeFileSync(join(out,name),`fixture ${name}`);
   const archive=join(out,"Moss_1.2.3_aarch64.app.tar.gz"),rawSig=join(keys,"signature");
   execFileSync("minisign",["-S","-s",join(keys,"key"),"-m",archive,"-x",rawSig,"-t","version:1.2.3"],{stdio:"ignore"});
   const signature=readFileSync(rawSig).toString("base64");writeFileSync(archive+".sig",signature);
   writeFileSync(join(out,"latest.json"),JSON.stringify({version:"1.2.3",platforms:{"darwin-aarch64":{signature,url:"https://github.com/hoomanaskari/moss-mail/releases/download/v1.2.3/Moss_1.2.3_aarch64.app.tar.gz"}}}));
   if(artifactKind) {
    const path=join(out,"Moss_1.2.3_aarch64.dmg");
    rmSync(path); if(artifactKind==="directory")mkdirSync(path); else writeFileSync(path,"");
   }
  }
  return "test-tool-version\n";
 }};
 return {root,git,sha,calls,logs,io,setTamperProvenance:(value:string)=>{tamperProvenance=value;},setArchitecture:(value:string)=>{architecture=value;},setArtifactKind:(value:string)=>{artifactKind=value;},setFailure:(value:string)=>{failure=value;},setMutate:(value:string)=>{mutate=value;},setTag:(value:string)=>{remoteTag=value;},setGithubTags:(value:string)=>{githubTags=value;}};
}
describe("release command",()=>{
 it("stops if a tool version cannot be recorded",async()=>{
  const f=fixture();const run=f.io.run;f.io.run=(command,args,options)=>command==="rustc"?"":run(command,args,options);
  await expect(release([],f.root,f.io)).rejects.toThrow("record rustc version");
  expect(f.calls.some(c=>c.command==="sh")).toBe(false);
 });
 it("requires the updater public key in the captured configuration",async()=>{
  const f=fixture();writeFileSync(join(f.root,"src-tauri/tauri.conf.json"),JSON.stringify({version:"1.2.3"}));
  await expect(release(["--local"],f.root,f.io)).rejects.toThrow("updater public key");
 });
 it("refuses publication when update signature verification fails",async()=>{
  const f=fixture();const run=f.io.run;f.io.run=(command,args,options)=>{if(command==="minisign"&&args[0]==="-V")throw new Error("signature rejected");return run(command,args,options);};
  await expect(release([],f.root,f.io)).rejects.toThrow("signature rejected");
  expect(f.calls.some(c=>c.command==="gh"&&c.args[0]==="release")).toBe(false);
 });
 it("does not give build steps publishing or signing credentials",async()=>{
  const f=fixture();Object.assign(f.io.env,{SSH_AUTH_SOCK:"agent",MOSS_UPDATER_KEY:"signing-key-path",MOSS_NOTARY_PROFILE:"notary",APPLE_SIGNING_IDENTITY:"identity"});
  await release([],f.root,f.io);
  for(const c of f.calls.filter(c=>c.command==="pnpm"))for(const key of ["GH_TOKEN","GITHUB_TOKEN","SSH_AUTH_SOCK","MOSS_UPDATER_KEY","MOSS_NOTARY_PROFILE","APPLE_SIGNING_IDENTITY"])expect(c.env[key]).toBeUndefined();
  const packaging=f.calls.find(c=>c.command==="sh")!;expect(packaging.env.MOSS_UPDATER_KEY).toBe("signing-key-path");expect(packaging.env.GH_TOKEN).toBeUndefined();expect(packaging.env.SSH_AUTH_SOCK).toBeUndefined();
  expect(f.calls.find(c=>c.command==="gh"&&c.args[0]==="release")?.env.GH_TOKEN).toBe("synthetic-test-token");
 });
 it.each(["success","check-failed"])("cleanup failure preserves %s outcome",async outcome=>{
  const f=fixture();if(outcome==="check-failed")f.setFailure("check");
  const actualFs=await vi.importActual<typeof import("node:fs")>("node:fs");
  vi.mocked(rmSync).mockImplementation((path,options)=>{if(String(path).includes("/moss-release-")){folders.push(String(path));throw new Error("cleanup failed");}actualFs.rmSync(path,options);});
  if(outcome==="success")await expect(release([],f.root,f.io)).resolves.toHaveProperty("output");
  else await expect(release([],f.root,f.io)).rejects.toThrow("injected command failure");
  expect(f.logs.some(line=>line.includes("Could not remove temporary release directory"))).toBe(true);
 });
 it("rechecks tags after the long build before creating a draft",async()=>{
  const f=fixture();const run=f.io.run;f.io.run=(command,args,options)=>{const result=run(command,args,options);if(command==="sh")f.setTag(`${f.sha}\trefs/tags/v1.2.3`);return result;};
  await expect(release([],f.root,f.io)).rejects.toThrow("already exists");
  expect(f.calls.some(c=>c.command==="gh"&&c.args[0]==="release")).toBe(false);
 });
 it.each(["check","package"])("refuses provenance changed during %s",async phase=>{
  const f=fixture();f.setTamperProvenance(phase);
  await expect(release([],f.root,f.io)).rejects.toThrow("provenance changed");
  expect(f.calls.some(c=>c.command==="gh"&&c.args[0]==="release")).toBe(false);
 });
 it("rejects unsupported hosts, branches and upstream mismatches",async()=>{
  const f=fixture();f.setArchitecture("x86_64");await expect(release([],f.root,f.io)).rejects.toThrow("Apple silicon");
  f.setArchitecture("arm64");f.git("checkout","-qb","feature");await expect(release([],f.root,f.io)).rejects.toThrow("from main");
  f.git("checkout","-q","main");writeFileSync(join(f.root,"main.ts"),"new");f.git("add",".");f.git("commit","-qm","new");
  await expect(release([],f.root,f.io)).rejects.toThrow("differs from origin/main");
 });
 it("rejects missing version values, empty notes and local tags",async()=>{
  const f=fixture();writeFileSync(join(f.root,"src-tauri/tauri.conf.json"),"{}");
  await expect(release(["--local"],f.root,f.io)).rejects.toThrow("version");
  f.git("checkout","--","src-tauri/tauri.conf.json");writeFileSync(join(f.root,"docs/releases/1.2.3.md")," ");
  await expect(release(["--local"],f.root,f.io)).rejects.toThrow("notes are empty");
  f.git("checkout","--","docs/releases/1.2.3.md");f.git("tag","v1.2.3");
  await expect(release([],f.root,f.io)).rejects.toThrow("already exists");
  await expect(release(["--local","unexpected"],f.root,f.io)).rejects.toThrow("Usage");
 });
 it.each(["directory","empty"])("refuses a %s artifact",async kind=>{
  const f=fixture();f.setArtifactKind(kind);
  await expect(release([],f.root,f.io)).rejects.toThrow(kind==="directory"?"regular file":"empty");
  expect(f.calls.some(c=>c.command==="gh"&&c.args[0]==="release")).toBe(false);
 });
 it("refuses artifacts corrupted during copying",async()=>{
  const f=fixture();vi.mocked(copyFileSync).mockImplementationOnce((_source,destination)=>{writeFileSync(destination,"corrupted");});
  await expect(release([],f.root,f.io)).rejects.toThrow("changed during copy");
  expect(f.calls.some(c=>c.command==="gh"&&c.args[0]==="release")).toBe(false);
 });
 it.each(["working","different-commit"])("refuses %s provenance for production",async kind=>{
  const f=fixture();const actual=await vi.importActual<typeof import("./release-source")>("./release-source.ts");
  vi.mocked(snapshotSource).mockImplementationOnce((entries,destination)=>actual.snapshotSource(entries,destination,kind==="working"?{kind:"working"}:{kind:"commit",sha:"b".repeat(40)}));
  await expect(release([],f.root,f.io)).rejects.toThrow("matching commit provenance");
  expect(f.calls.some(c=>c.command==="gh"&&c.args[0]==="release")).toBe(false);
 });
 it("validates immutable source, records provenance, and drafts against its exact commit",async()=>{
  const f=fixture();const result=await release([],f.root,f.io);
  const buildCalls=f.calls.filter(c=>c.command==="pnpm"||c.command==="sh");
  expect(buildCalls.filter(c=>["install","check"].includes(c.args[0])||c.command==="sh").map(c=>c.args[0])).toEqual(["install","check","scripts/package-release.sh"]);
  expect(buildCalls.find(c=>c.args[0]==="install")?.args).toEqual(["install","--frozen-lockfile"]);
  const snapshot=buildCalls.find(c=>c.args[0]==="check")!.cwd;
  expect(snapshot).not.toBe(f.root);expect(existsSync(snapshot)).toBe(false);
  const manifest=JSON.parse(readFileSync(join(result.output,"release-manifest.json"),"utf8"));
  expect(manifest.source.origin).toEqual({kind:"commit",sha:f.sha});expect(manifest.artifacts).toHaveLength(4);
  expect(manifest.tools.rustc).toBe("test-tool-version");expect(JSON.stringify(manifest)).not.toContain("synthetic-test-token");
  const verification=f.calls.find(c=>c.command==="minisign"&&c.args[0]==="-V")!;
  expect(verification.args[verification.args.indexOf("-m")+1]).toBe(join(result.output,"Moss_1.2.3_aarch64.app.tar.gz"));
  const publish=f.calls.find(c=>c.command==="gh"&&c.args[0]==="release")!;
  expect(publish.env.GH_HOST).toBe("github.com");expect(publish.args).toContain("--draft");expect(publish.args[publish.args.indexOf("--target")+1]).toBe(f.sha);
  expect(publish.args).toContain(join(result.output,"release-manifest.json"));
  for(const call of f.calls){expect(call.env.NODE_OPTIONS).toBeUndefined();expect(call.env.GIT_DIR).toBeUndefined();}
  expect(buildCalls.find(c=>c.args[0]==="check")?.env.CARGO_TARGET_DIR).toBe(join(snapshot,"target"));
 });
 it("keeps dirty local rehearsals separate and never invokes GitHub or remote Git",async()=>{
  const f=fixture();writeFileSync(join(f.root,"main.ts"),"dirty");const result=await release(["--local"],f.root,f.io);
  expect(result.output).toContain("/local/");expect(f.calls.some(c=>c.command==="gh"||c.args[0]==="fetch"||c.args[0]==="ls-remote")).toBe(false);
  expect(f.calls.find(c=>c.command==="sh")?.args).toEqual(["scripts/package-release.sh","--local"]);
  const manifest=JSON.parse(readFileSync(join(result.output,"release-manifest.json"),"utf8"));expect(manifest.source.origin).toEqual({kind:"working"});expect(manifest.rehearsal).toBe(true);
 });
 it.each(["install","check","package"])("does not publish after failed %s and cleans the snapshot",async phase=>{
  const f=fixture();f.setFailure(phase);await expect(release([],f.root,f.io)).rejects.toThrow("injected command failure");
  expect(f.calls.some(c=>c.command==="gh"&&c.args[0]==="release")).toBe(false);
  const build=f.calls.find(c=>c.command==="pnpm"&&c.args[0]==="install")!;expect(existsSync(build.cwd)).toBe(false);
 });
 it.each(["install","check","package"])("does not publish when %s changes source",async phase=>{
  const f=fixture();f.setMutate(phase);await expect(release([],f.root,f.io)).rejects.toThrow("source changed");
  expect(f.calls.some(c=>c.command==="gh"&&c.args[0]==="release")).toBe(false);
 });
 it("rejects dirty production source before installing",async()=>{
  const f=fixture();writeFileSync(join(f.root,"main.ts"),"dirty");await expect(release([],f.root,f.io)).rejects.toThrow("uncommitted");
  expect(f.calls.some(c=>c.command==="pnpm")).toBe(false);
 });
 it("rejects invalid arguments and unsafe versions",async()=>{
  const f=fixture();await expect(release(["--unknown"],f.root,f.io)).rejects.toThrow("Usage");
  writeFileSync(join(f.root,"src-tauri/tauri.conf.json"),JSON.stringify({version:"../../outside"}));
  await expect(release(["--local"],f.root,f.io)).rejects.toThrow("version");expect(f.calls.some(c=>c.command==="sh")).toBe(false);
 });
 it("rejects existing remote tags and GitHub releases",async()=>{
  const f=fixture();f.setTag(`${f.sha}\trefs/tags/v1.2.3`);await expect(release([],f.root,f.io)).rejects.toThrow("already exists");
  f.setTag("");f.setGithubTags("v1.2.3\n");await expect(release([],f.root,f.io)).rejects.toThrow("already exists");
  expect(f.calls.some(c=>c.command==="sh")).toBe(false);
 });
});
