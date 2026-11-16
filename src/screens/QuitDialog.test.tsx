import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
const invoke=vi.fn(async(..._args:unknown[])=>{});
vi.mock("@tauri-apps/api/core",()=>({invoke:(...args:unknown[])=>invoke(...args)}));
import { useQuit } from "../services/quit";
import { useStore } from "../state/store";
import { QuitDialog } from "./QuitDialog";
afterEach(()=>{useQuit.setState({token:undefined,exiting:false,checking:false,blockers:[],error:undefined});invoke.mockClear();});
it("shows nothing when quitting has not been requested",()=>{render(<QuitDialog/>);expect(screen.queryByRole("dialog")).not.toBeInTheDocument();});
it("keeps typing outside the modal blocked while checking and offers cancellation",async()=>{
 useQuit.setState({token:"1",checking:true});render(<QuitDialog/>);
 expect(screen.getByRole("dialog",{name:"Saving before quitting"})).toHaveAttribute("open");
 expect(screen.queryByRole("button",{name:"Quit anyway"})).not.toBeInTheDocument();
 await userEvent.click(screen.getByRole("button",{name:"Keep working"}));
 expect(invoke).toHaveBeenCalledWith("quit_cancel",{token:"1"});
 expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
it("shows named blockers and makes discarding unsaved work an explicit choice",async()=>{
 useQuit.setState({token:"2",checking:false,blockers:[{id:"draft",subject:"A proposal",reason:"Disk full"}],error:"Connection lost"});render(<QuitDialog/>);
 expect(screen.getByRole("dialog",{name:"Drafts are not confirmed saved"})).toBeVisible();
 expect(screen.getByText("A proposal")).toBeVisible();expect(screen.getByText("Disk full")).toBeVisible();expect(screen.getByRole("alert")).toHaveTextContent("Connection lost");
 await userEvent.click(screen.getByRole("button",{name:"Quit anyway"}));
 expect(invoke).toHaveBeenCalledWith("quit_ready",{token:"2"});
});
it("returns to Drafts for review instead of quitting",async()=>{
 useQuit.setState({token:"3",checking:false});render(<QuitDialog/>);
 await userEvent.click(screen.getByRole("button",{name:"Review drafts"}));
 await waitFor(()=>expect(useStore.getState().view).toBe("drafts"));
 expect(invoke).not.toHaveBeenCalledWith("quit_ready",expect.anything());
});
it("treats Escape as keep working and prevents app shortcuts beneath the dialog",()=>{
 useQuit.setState({token:"4",checking:true});render(<QuitDialog/>);
 const background=vi.fn();window.addEventListener("keydown",background);
 fireEvent.keyDown(screen.getByRole("button",{name:"Keep working"}),{key:"k",ctrlKey:true});
 expect(background).not.toHaveBeenCalled();window.removeEventListener("keydown",background);
 fireEvent(screen.getByRole("dialog"),new Event("cancel",{bubbles:true,cancelable:true}));
 expect(useQuit.getState().token).toBeUndefined();
});

it("does not reopen an already modal dialog when a new quit token arrives",()=>{
 const modal=vi.spyOn(HTMLDialogElement.prototype,"showModal");
 useQuit.setState({token:"first",checking:false});render(<QuitDialog/>);
 act(()=>useQuit.setState({token:"second",checking:true}));
 expect(modal).toHaveBeenCalledOnce();
 expect(screen.getByRole("dialog")).toHaveAttribute("open");
});

it("cycles Tab and Shift-Tab within its buttons",async()=>{
 useQuit.setState({token:"focus",checking:false});render(<QuitDialog/>);
 const first=screen.getByRole("button",{name:"Keep working"});const last=screen.getByRole("button",{name:"Quit anyway"});
 first.focus();await userEvent.keyboard("{Shift>}{Tab}{/Shift}");expect(last).toHaveFocus();
 await userEvent.keyboard("{Tab}");expect(first).toHaveFocus();
 await userEvent.keyboard("{Tab}");expect(screen.getByRole("button",{name:"Review drafts"})).toHaveFocus();
});

it("cannot dismiss the modal after the exit handoff starts", async () => {
 useQuit.setState({token:"exiting",checking:false});render(<QuitDialog/>);
 await userEvent.click(screen.getByRole("button",{name:"Quit anyway"}));
 expect(screen.queryByRole("button",{name:"Keep working"})).not.toBeInTheDocument();
 fireEvent.keyDown(screen.getByRole("dialog"),{key:"Tab"});
 fireEvent(screen.getByRole("dialog"),new Event("cancel",{bubbles:true,cancelable:true}));
 expect(useQuit.getState().token).toBe("exiting");
 expect(invoke).not.toHaveBeenCalledWith("quit_cancel",expect.anything());
});

it("reviews authoritative local drafts even when account sync is unavailable",async()=>{
 const {fakeMail}=await import("../test/fakeMail");
 const {useDrafts,draftsSaved}=await import("../state/drafts");
 vi.spyOn(console,"warn").mockImplementation(()=>{});
 const {backend}=fakeMail({syncNow:vi.fn(async()=>{throw new Error("Signed out");}),drafts:vi.fn(async()=>[])});
 useDrafts.setState({status:{deleted:"failed"}});
 useQuit.setState({token:"offline-delete",checking:false});render(<QuitDialog/>);
 await userEvent.click(screen.getByRole("button",{name:"Review drafts"}));await draftsSaved();
 expect(backend.drafts).toHaveBeenCalledOnce();expect(useDrafts.getState().status).toEqual({});
});
