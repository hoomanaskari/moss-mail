import { afterEach, expect, it, vi } from "vitest";
const invoke=vi.fn(async (..._args: unknown[])=>{});
const listen=vi.fn();const settle=vi.fn(async()=>[]);
vi.mock("@tauri-apps/api/core",()=>({invoke:(...args:unknown[])=>invoke(...args)}));
vi.mock("@tauri-apps/api/event",()=>({listen:(...args:unknown[])=>listen(...args)}));
vi.mock("../lib/platform",()=>({isTauri:true}));
vi.mock("../state/drafts",()=>({settleDraftsForQuit:()=>settle(),currentDraftQuitBlockers:()=>[]}));
import { keepWorking, protectQuit, quitAnyway, useQuit, watchQuit } from "./quit";
afterEach(()=>{useQuit.setState({token:undefined,exiting:false,checking:false,blockers:[],error:undefined});invoke.mockClear();settle.mockReset().mockResolvedValue([]);});
async function start() {
 let handler!:(event:{payload:string})=>void;
 const stop=vi.fn();listen.mockImplementation(async (_name:string,callback:typeof handler)=>{handler=callback;return stop;});
 const cleanup=watchQuit();await Promise.resolve();return {handler,cleanup,stop};
}
it("confirms quit only after draft settlement",async()=>{
 const {handler,cleanup}=await start();handler({payload:"1"});
 await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("quit_ready",{token:"1"}));
 expect(settle).toHaveBeenCalledOnce();cleanup();
});
it("keeps the app open with named unsaved drafts, then permits explicit quit anyway",async()=>{
 settle.mockResolvedValue([{id:"draft",subject:"Important",reason:"Disk full"}] as never);
 const {handler,cleanup}=await start();handler({payload:"2"});
 await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("quit_blocked",{token:"2"}));
 expect(useQuit.getState().blockers[0].reason).toBe("Disk full");
 expect(invoke).not.toHaveBeenCalledWith("quit_ready",expect.anything());
 await quitAnyway();expect(invoke).toHaveBeenCalledWith("quit_ready",{token:"2"});cleanup();
});
it("never allows an old settlement after the user chose to keep working",async()=>{
 let finish!:(value:never[])=>void;settle.mockImplementationOnce(()=>new Promise((resolve)=>{finish=resolve;}));
 const {handler,cleanup}=await start();handler({payload:"3"});await keepWorking();finish([]);await Promise.resolve();await Promise.resolve();
 expect(invoke).not.toHaveBeenCalledWith("quit_ready",expect.anything());expect(useQuit.getState().token).toBeUndefined();
 expect(invoke).toHaveBeenCalledWith("quit_cancel",{token:"3"});cleanup();
});
it("blocks quit during a user-initiated native operation and releases it on failure",async()=>{
 let reject!:(error:Error)=>void;
 const work=protectQuit("A send is still being confirmed",()=>new Promise((_resolve,fail)=>{reject=fail;}));
 const caught=work.catch(()=>{});
 const {handler,cleanup}=await start();handler({payload:"4"});
 await vi.waitFor(()=>expect(useQuit.getState().blockers).toEqual([expect.objectContaining({reason:"A send is still being confirmed"})]));
 reject(new Error("refused"));await caught;await keepWorking();handler({payload:"5"});
 await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("quit_ready",{token:"5"}));cleanup();
});

it("keeps the gate closed when settling or native acknowledgement fails",async()=>{
 settle.mockRejectedValueOnce(new Error("storage unavailable"));
 invoke.mockRejectedValueOnce(new Error("native reply lost"));
 const {handler,cleanup}=await start();handler({payload:"6"});
 await vi.waitFor(()=>expect(useQuit.getState().error).toBe("storage unavailable"));
 expect(useQuit.getState().checking).toBe(false);expect(invoke).not.toHaveBeenCalledWith("quit_ready",expect.anything());cleanup();
});
it("does not publish a stale failure after another quit request",async()=>{
 let fail!:(error:Error)=>void;settle.mockImplementationOnce(()=>new Promise((_ok,reject)=>{fail=reject;}));
 const {handler,cleanup}=await start();handler({payload:"old"});handler({payload:"new"});
 await vi.waitFor(()=>expect(invoke).toHaveBeenCalledWith("quit_ready",{token:"new"}));
 fail(new Error("old failure"));await Promise.resolve();await Promise.resolve();expect(useQuit.getState().error).toBeUndefined();cleanup();
});
it("a slow save times out to a blocker, never to permission to exit",async()=>{
 vi.useFakeTimers();
 let finish!:(value:never[])=>void;settle.mockImplementationOnce(()=>new Promise((resolve)=>{finish=resolve;}));
 const {handler,cleanup}=await start();
 try {
  handler({payload:"slow"});await vi.advanceTimersByTimeAsync(10000);
  expect(invoke).toHaveBeenCalledWith("quit_blocked",{token:"slow"});
  expect(useQuit.getState().blockers[0].reason).toContain("longer than expected");
  finish([]);await Promise.resolve();expect(invoke).not.toHaveBeenCalledWith("quit_ready",expect.anything());
 } finally {cleanup();vi.useRealTimers();}
});
it("cleans up a listener that resolves after unmount and ignores queued events",async()=>{
 let register!:(stop:()=>void)=>void;let handler!:(event:{payload:string})=>void;
 listen.mockImplementationOnce((_name:string,callback:typeof handler)=>{handler=callback;return new Promise((resolve)=>{register=resolve;});});
 const stop=vi.fn();const cleanup=watchQuit();cleanup();register(stop);await Promise.resolve();handler({payload:"late"});
 expect(stop).toHaveBeenCalledOnce();expect(settle).not.toHaveBeenCalled();
});
it("reports listener setup failure without opening an unsafe exit path",async()=>{
 const warn=vi.spyOn(console,"warn").mockImplementation(()=>{});listen.mockRejectedValueOnce(new Error("No listener"));
 const cleanup=watchQuit();await vi.waitFor(()=>expect(warn).toHaveBeenCalledWith("Moss quit listener:","No listener"));
 expect(invoke).not.toHaveBeenCalledWith("quit_ready",expect.anything());cleanup();
});
it("shows a failed explicit quit acknowledgement and ignores one after cancellation",async()=>{
 await quitAnyway();expect(invoke).not.toHaveBeenCalled();
 useQuit.setState({token:"explicit"});invoke.mockRejectedValueOnce(new Error("No reply"));await quitAnyway();
 expect(useQuit.getState().error).toBe("No reply");expect(useQuit.getState().checking).toBe(false);
 let reject!:(error:Error)=>void;invoke.mockImplementationOnce(()=>new Promise((_resolve,fail)=>{reject=fail;}));
 const pending=quitAnyway();useQuit.setState({token:undefined,exiting:false,error:undefined});reject(new Error("late"));await pending;
 expect(useQuit.getState().error).toBeUndefined();
});
it("logs cancellation failure but never turns it into an exit",async()=>{
 const warn=vi.spyOn(console,"warn").mockImplementation(()=>{});
 useQuit.setState({token:"cancel"});invoke.mockRejectedValueOnce(new Error("No reply"));await keepWorking();
 expect(warn).toHaveBeenCalledWith("Moss quit cancellation:","No reply");expect(useQuit.getState().token).toBeUndefined();
});

it.each(["automatic", "explicit", "blocked"])("dismisses a quit dialog when native rejects its stale token: %s",async(mode)=>{
 invoke.mockResolvedValueOnce(false as never);
 if(mode==="explicit") {useQuit.setState({token:"stale"});await quitAnyway();}
 else {
  if(mode==="blocked") settle.mockResolvedValueOnce([{id:"draft",subject:"Draft",reason:"Unsaved"}] as never);
  const {handler,cleanup}=await start();handler({payload:"stale"});
  await vi.waitFor(()=>expect(useQuit.getState().token).toBeUndefined());cleanup();
 }
 expect(useQuit.getState().token).toBeUndefined();expect(useQuit.getState().checking).toBe(false);
});
