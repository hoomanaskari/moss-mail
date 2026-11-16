import { expect, it } from "vitest";
import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { verifyUpdate } from "./release-update";
it.each(["--local", "--production"])("packages %s with locked dependencies and provenance without publishing",mode=>{
 const root=mkdtempSync(join(tmpdir(),"moss-package-test-"));
 try {
  for(const dir of ["scripts","bin","docs/releases","src-tauri","target"])mkdirSync(join(root,dir),{recursive:true});
  for(const name of ["package-release.sh","signing-identity.sh"])copyFileSync(join("scripts",name),join(root,"scripts",name));
  writeFileSync(join(root,"src-tauri/tauri.conf.json"),JSON.stringify({version:"1.2.3"}));
  writeFileSync(join(root,"docs/releases/1.2.3.md"),"Notes");execFileSync(process.execPath,[join(process.cwd(),"node_modules/@tauri-apps/cli/tauri.js"),"signer","generate","--ci","--password","","--write-keys",join(root,"key")],{stdio:"ignore"});
  writeFileSync(join(root,"target/moss-release-source.json"),JSON.stringify({source:{origin:mode==="--local"?{kind:"working"}:{kind:"commit",sha:"a".repeat(40)}}}));
  const runner=join(root,"bin/fake-tool");
  writeFileSync(runner,`#!${process.execPath}
const fs=require('fs'),path=require('path');const tool=path.basename(process.argv[1]),args=process.argv.slice(2);
fs.appendFileSync(process.env.TRACE,JSON.stringify({tool,args})+'\\n');
if(tool==='uname')console.log(args[0]==='-m'?'arm64':'Darwin');
if(tool==='pnpm'&&args.includes('build'))fs.mkdirSync('target/release/bundle/macos/Moss.app',{recursive:true});
if(tool==='pnpm'&&args.includes('sign'))require('child_process').execFileSync(process.execPath,[process.env.REAL_TAURI_CLI,...args.slice(args.indexOf('tauri')+1)],{stdio:'pipe'});
if(tool==='tar')fs.writeFileSync(args[1],'fixture archive');
if(tool==='create-dmg')fs.writeFileSync(args.at(-2),'fixture dmg');
if(tool==='ditto'){if(args.includes('-c'))fs.writeFileSync(args.at(-1),'fixture zip');else fs.mkdirSync(args.at(-1),{recursive:true});}
`);chmodSync(runner,0o755);
  for(const tool of ["uname","pnpm","codesign","ditto","create-dmg","tar","gh","xcrun","spctl"])symlinkSync(runner,join(root,"bin",tool));
  const trace=join(root,"trace.jsonl");
  execFileSync("sh",["scripts/package-release.sh",mode],{cwd:root,env:{...process.env,PATH:`${join(root,"bin")}:${process.env.PATH}`,APPLE_SIGNING_IDENTITY:"synthetic",MOSS_UPDATER_KEY:join(root,"key"),TRACE:trace,REAL_TAURI_CLI:join(process.cwd(),"node_modules/@tauri-apps/cli/tauri.js")}});
  const calls=readFileSync(trace,"utf8").trim().split("\n").map(line=>JSON.parse(line));
  const build=calls.find(c=>c.tool==="pnpm"&&c.args.includes("build"));
  expect(build.args).toContain("--locked");
  const config=JSON.parse(build.args[build.args.indexOf("--config")+1]);
  expect(config.bundle.resources).toEqual({"../target/moss-release-source.json":"release-source.json"});
  expect(calls.some(c=>c.tool==="gh")).toBe(false);
  const appleCalls=calls.filter(c=>c.tool==="xcrun");
  expect(appleCalls.filter(c=>c.args[0]==="notarytool"&&c.args[1]==="submit")).toHaveLength(mode==="--local"?0:2);
  expect(appleCalls.filter(c=>c.args[0]==="stapler"&&c.args[1]==="staple")).toHaveLength(mode==="--local"?0:2);
  const out=join(root,"target/release/moss-release/1.2.3");
  expect(existsSync(join(out,"latest.json"))).toBe(true);
  expect(()=>verifyUpdate({archive:join(out,"Moss_1.2.3_aarch64.app.tar.gz"),signature:readFileSync(join(out,"Moss_1.2.3_aarch64.app.tar.gz.sig"),"utf8"),publicKey:readFileSync(join(root,"key.pub"),"utf8"),version:"1.2.3",url:"https://github.com/hoomanaskari/moss-mail/releases/download/v1.2.3/Moss_1.2.3_aarch64.app.tar.gz",manifest:JSON.parse(readFileSync(join(out,"latest.json"),"utf8"))},args=>execFileSync("minisign",args,{encoding:"utf8",stdio:["ignore","pipe","pipe"]}))).not.toThrow();
 } finally {rmSync(root,{recursive:true,force:true});}
});
