import { expect, it, vi } from "vitest";
const invoke=vi.fn();let refusePrepared=false;let listener!:(event:{payload:string})=>void;let prepared:(event:{payload:string})=>void=()=>{};
vi.mock("@tauri-apps/api/core",()=>({invoke:(...args:unknown[])=>invoke(...args)}));
vi.mock("@tauri-apps/api/event",()=>({listen:async(name:string,handler:typeof listener)=>{if(name==="moss://quit-requested")listener=handler;
 if(name==="moss://send-prepared") {if(refusePrepared)throw new Error("Listener unavailable");prepared=handler;}return ()=>{};}}));
vi.mock("../lib/platform",()=>({isTauri:true,isMac:false,modKey:"Control",deviceName:"computer"}));
import { setAsk, type AskAnswer } from "./ask";
import { resetAskPanel, useAskPanel } from "../state/askPanel";
import { useMossWriting } from "../state/mossWriting";
import { setReplies } from "./replies";
import { tauriReplies } from "./tauriReplies";
import { tauriAsk } from "./tauriAsk";
import { whileStreaming } from "./tauriText";
import { tauriMail } from "./tauriMail";
import { keepWorking, useQuit, watchQuit } from "./quit";
import { resetStore, thread, work } from "../test/fixtures";
import { draftsSaved, keepDraft, useDrafts, watchDrafts } from "../state/drafts";
import { fakeMail } from "../test/fakeMail";
import { sendBeatMs, useStore } from "../state/store";

it("blocks quitting from the moment a native send starts, before its first IPC reply",async()=>{
 resetStore([],[work]);useStore.getState().compose("new");useStore.getState().updateDraft({body:"Send this"});
 let finish!:(value:boolean)=>void;
 invoke.mockImplementation(async(command:string)=> command==="send_after" ? new Promise<boolean>((resolve)=>{finish=resolve;}) : undefined);
 const stop=watchQuit();await Promise.resolve();
 try {
  const sending=tauriMail.sendAfter("hold",useStore.getState().draft!,1000);
  listener({payload:"1"});
  await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("quit_blocked",{token:"1"}));
  expect(useQuit.getState().blockers.some((item)=>item.reason.includes("send"))).toBe(true);
  finish(true);await sending;await keepWorking();listener({payload:"2"});
  await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("quit_ready",{token:"2"}));
 } finally {stop();}
});

it.each(["reschedule", "unschedule"])("blocks quit until a %s request is confirmed",async(action)=>{
 let reject!:(reason:Error)=>void;
 invoke.mockClear();invoke.mockImplementation(async(command:string)=>command.endsWith("schedule_send") ? new Promise((_resolve,fail)=>{reject=fail;}) : undefined);
 const stop=watchQuit();await Promise.resolve();
 try {
  const request=action==="reschedule" ? tauriMail.rescheduleSend("draft",123,"operation",1) : tauriMail.unscheduleSend("draft","operation");
  const caught=request.catch(()=>{});listener({payload:"schedule"});
  await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("quit_blocked",{token:"schedule"}));
  expect(useQuit.getState().blockers[0].reason).toContain("schedule");
  reject(new Error("refused"));await caught;
 } finally {stop();}
});

it.each(["ask", "writing"])("blocks quit while %s can still produce local edits",async(action)=>{
 let reject!:(reason:Error)=>void;invoke.mockClear();
 invoke.mockImplementation(async(command:string)=>["ask_moss","ai_reply"].includes(command) ? new Promise((_resolve,fail)=>{reject=fail;}) : undefined);
 const stop=watchQuit();await Promise.resolve();
 try {
  const pending=action==="ask" ? tauriAsk.askMoss("chat","Draft a reply") : whileStreaming("write",()=>{},()=>invoke("ai_reply"));
  const caught=pending.catch(()=>{});listener({payload:"ai"});
  await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("quit_blocked",{token:"ai"}));
  expect(useQuit.getState().blockers[0].reason).toContain("AI");
  reject(new Error("stopped"));await caught;
 } finally {stop();}
});

it("allows quit during Undo only after the matching native delivery is durably held",async()=>{
 resetStore([],[work]);useStore.getState().compose("new");useStore.getState().updateDraft({body:"Held email"});
 let finish!:(value:boolean)=>void;invoke.mockClear();invoke.mockImplementation(async(command:string)=>command==="send_after" ? new Promise<boolean>((resolve)=>{finish=resolve;}) : undefined);
 const stop=watchQuit();await Promise.resolve();
 try {
  const sending=tauriMail.sendAfter("held-operation",useStore.getState().draft!,20000);
  await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("send_after",expect.anything()));
  prepared({payload:"another-operation"});listener({payload:"before"});
  await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("quit_blocked",{token:"before"}));
  await keepWorking();prepared({payload:"held-operation"});listener({payload:"after"});
  await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("quit_ready",{token:"after"}));
  finish(true);await sending;
 } finally {stop();}
});

it("keeps quit blocked until the send reply if the acceptance listener cannot register",async()=>{
 resetStore([],[work]);useStore.getState().compose("new");useStore.getState().updateDraft({body:"Keep this"});
 let finish!:(sent:boolean)=>void;
 invoke.mockClear();invoke.mockImplementation(async(command:string)=>command==="send_after"?new Promise<boolean>(resolve=>{finish=resolve;}):undefined);
 const stop=watchQuit();await Promise.resolve();refusePrepared=true;
 try {
  const sending=tauriMail.sendAfter("listener-unavailable",useStore.getState().draft!,20000);
  await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("send_after",expect.anything()));
  listener({payload:"listener-failed"});
  await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("quit_blocked",{token:"listener-failed"}));
  finish(true);await sending;await keepWorking();listener({payload:"reply-confirmed"});
  await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("quit_ready",{token:"reply-confirmed"}));
 } finally {refusePrepared=false;stop();}
});

it("does not retry a failed save once native owns the exact sent content",async()=>{
 resetStore([],[work]);vi.spyOn(console,"warn").mockImplementation(()=>{});
 const save=vi.fn(async()=>{throw new Error("temporary write failure");});
 const {backend}=fakeMail({saveDraft:save,draftSaveReceipt:async()=>undefined});
 useStore.getState().compose("new");useStore.getState().updateDraft({body:"Native now owns this"});
 const draft=useStore.getState().draft!;keepDraft(draft);await draftsSaved();
 expect(useDrafts.getState().unsaved[draft.id]).toBeDefined();
 let finish!:(sent:boolean)=>void;invoke.mockClear();
 invoke.mockImplementation(async(command:string)=>command==="send_after"?new Promise<boolean>(resolve=>{finish=resolve;}):undefined);
 const stop=watchQuit();await Promise.resolve();
 try {
  const sending=tauriMail.sendAfter("prepared-failed-save",draft,20000);
  await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("send_after",expect.anything()));
  listener({payload:"during-prepare"});
  await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("quit_blocked",{token:"during-prepare"}));
  expect(save).toHaveBeenCalledOnce();await keepWorking();
  prepared({payload:"prepared-failed-save"});listener({payload:"quit-prepared"});
  await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("quit_ready",{token:"quit-prepared"}));
  expect(save).toHaveBeenCalledOnce();expect(backend.keepLocalDraftRecovery).not.toHaveBeenCalled();
  expect(useDrafts.getState().unsaved[draft.id]).toBeUndefined();finish(true);await sending;
 } finally {stop();}
});

it.each([0,1,2,3,4,5,6,7,8,9,10,11,12])("never grants quit between an AI reply and its composer edit, offset %i",async(ticks)=>{
 resetStore([],[work]);useMossWriting.getState().reset();setReplies(tauriReplies);
 const save=vi.fn(async(draft: import("../state/drafts").SavedDraft)=>({...draft,revision:"saved"}));
 fakeMail({saveDraft:save});const unwatch=watchDrafts(useStore);
 useStore.getState().compose("new");useStore.getState().updateDraft({body:"Original"});
 let answer!:(text:string)=>void;invoke.mockClear();
 let verdict: { body?: string; unsaved: string[]; writing?: string } | undefined;
 invoke.mockImplementation(async(command:string)=>{
  if(command==="quit_ready") verdict={body:save.mock.calls.at(-1)?.[0].body,unsaved:Object.keys(useDrafts.getState().unsaved),writing:useMossWriting.getState().status};
  return command==="ai_reply"?new Promise<string>(resolve=>{answer=resolve;}):undefined;
 });
 const stop=watchQuit();await Promise.resolve();
 try {
  const writing=useMossWriting.getState().write({job:{kind:"shorter"},lead:"Shortened"});
  await vi.waitFor(()=>expect(answer).toBeDefined());
  listener({payload:`ai-race-${ticks}`});
  for(let i=0;i<ticks;i++) await Promise.resolve();
  answer("New AI text");await writing;
  await vi.waitFor(()=>expect(invoke.mock.calls.some(([name])=>name==="quit_ready"||name==="quit_blocked")).toBe(true));
  if(verdict) {expect(verdict.body).toBe("New AI text");expect(verdict.unsaved).toEqual([]);expect(verdict.writing).not.toBe("writing");}
  else expect(useQuit.getState().blockers.length).toBeGreaterThan(0);
 } finally {stop();unwatch();}
});

it.each([0,1,2,3,4,5,6,7,8])("keeps returned Ask Moss drafts durable across quit, offset %i",async(ticks)=>{
 resetStore([thread("ask-thread")],[work]);resetAskPanel();
 const save=vi.fn(async(draft: import("../state/drafts").SavedDraft)=>({...draft,revision:"saved"}));
 fakeMail({saveDraft:save});let answer!:(value:AskAnswer)=>void;
 setAsk(tauriAsk);
 useStore.getState().open("ask-thread");useStore.getState().compose("reply");
 const draft={...useStore.getState().draft!,body:"AI reply",updatedAt:1};
 invoke.mockClear();invoke.mockImplementation(async(command:string)=>command==="ask_moss"?new Promise<AskAnswer>(resolve=>{answer=resolve;}):undefined);
 const stop=watchQuit();await Promise.resolve();
 try {
  const asking=useAskPanel.getState().ask("Draft a response");listener({payload:`ask-race-${ticks}`});
  for(let i=0;i<ticks;i++) await Promise.resolve();
  answer({text:"Here is a draft",sources:[],messages:[],drafts:[draft],stopped:false});await asking;
  await vi.waitFor(()=>expect(invoke.mock.calls.some(([name])=>name==="quit_ready"||name==="quit_blocked")).toBe(true));
  if(invoke.mock.calls.some(([name])=>name==="quit_ready")) expect(save.mock.calls.at(-1)?.[0].body).toBe("AI reply");
  else expect(useQuit.getState().blockers.length).toBeGreaterThan(0);
  await draftsSaved();expect(save).toHaveBeenCalledWith(expect.objectContaining({body:"AI reply"}));
 } finally {stop();}
});

it("does not duplicate a confirmed native delivery when its result is uncertain",async()=>{
 vi.useFakeTimers();resetStore([],[work]);vi.spyOn(console,"warn").mockImplementation(()=>{});
 const {backend}=fakeMail({sendAfter:tauriMail.sendAfter});
 useStore.getState().compose("new");useStore.getState().updateDraft({subject:"Prepared email",body:"Durable snapshot"});
 let reject!:(reason:Error)=>void;invoke.mockClear();
 invoke.mockImplementation(async(command:string)=>command==="send_after"?new Promise((_resolve,fail)=>{reject=fail;}):undefined);
 try {
  useStore.getState().send();await vi.advanceTimersByTimeAsync(sendBeatMs);
  const hold=invoke.mock.calls.find(([name])=>name==="send_after")![1].hold;
  prepared({payload:hold});reject(Object.assign(new Error("Check Sent"),{kind:"deliveryUnknown"}));
  await vi.advanceTimersByTimeAsync(0);await draftsSaved();
  expect(useDrafts.getState().volatile.size).toBe(0);expect(useDrafts.getState().recoveries.size).toBe(0);
  expect(backend.saveDraft).not.toHaveBeenCalled();expect(useStore.getState().toast?.text).toBe("Check Sent");
 } finally {vi.useRealTimers();}
});

it.each([["composer",false],["composer",true],["draft",false],["draft",true]] as const)("preserves cancellation content from %s only before native preparation: %s",async(surface,confirmed)=>{
 vi.useFakeTimers();resetStore([],[work]);let finish!:(sent:boolean)=>void;
 const save=vi.fn(async(draft:import("../state/drafts").SavedDraft)=>({...draft,revision:"saved"}));
 fakeMail({sendAfter:tauriMail.sendAfter,saveDraft:save});
 useStore.getState().compose("new");useStore.getState().updateDraft({subject:"Cancel",body:"Newest content"});
 const draft=useStore.getState().draft!;invoke.mockClear();
 invoke.mockImplementation(async(command:string)=>command==="send_after"?new Promise<boolean>(resolve=>{finish=resolve;}):undefined);
 try {
  if(surface==="composer") useStore.getState().send();else useStore.getState().sendDraft(draft);
  await vi.advanceTimersByTimeAsync(sendBeatMs);
  const hold=invoke.mock.calls.find(([name])=>name==="send_after")![1].hold;
  if(confirmed) prepared({payload:hold});finish(false);await vi.advanceTimersByTimeAsync(0);await draftsSaved();
  if(confirmed) expect(save).not.toHaveBeenCalled();else expect(save).toHaveBeenCalledWith(expect.objectContaining({body:"Newest content"}));
  expect(useDrafts.getState().volatile.size).toBe(0);
 } finally {vi.useRealTimers();}
});
it("a preparation acknowledgement during Undo prevents an unnecessary recovery on a lost cancellation reply",async()=>{
 vi.useFakeTimers();resetStore([],[work]);let sent!:(value:boolean)=>void;let cancel!:(error:Error)=>void;
 const {backend}=fakeMail({sendAfter:tauriMail.sendAfter,cancelSend:tauriMail.cancelSend});
 useStore.getState().compose("new");useStore.getState().updateDraft({subject:"Undo",body:"Native owns this"});invoke.mockClear();
 invoke.mockImplementation(async(command:string)=>command==="send_after"?new Promise<boolean>(resolve=>{sent=resolve;}):command==="cancel_send"?new Promise((_resolve,reject)=>{cancel=reject;}):undefined);
 try {
  useStore.getState().send();await vi.advanceTimersByTimeAsync(sendBeatMs);useStore.getState().undo();await vi.advanceTimersByTimeAsync(0);
  const hold=invoke.mock.calls.find(([name])=>name==="send_after")![1].hold;prepared({payload:hold});cancel(new Error("Lost reply"));
  await vi.advanceTimersByTimeAsync(0);sent(true);await vi.advanceTimersByTimeAsync(0);
  expect(backend.keepLocalDraftRecovery).not.toHaveBeenCalled();expect(useDrafts.getState().volatile.size).toBe(0);
 } finally {vi.useRealTimers();}
});
