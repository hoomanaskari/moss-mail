import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import type { BackupStatus } from "../../services/backups";
import { fakeMail } from "../../test/fakeMail";
import { BackupSettings } from "./BackupSettings";

afterEach(() => vi.useRealTimers());
const running = (cancelling = false): BackupStatus => ({state:"running", id:7, progress:{phase:"copying",copiedPages:128,totalPages:500}, cancelling});
const complete: BackupStatus = {state:"complete",id:7,bytes:1024,sha256:"a".repeat(64)};

it("creates a recovery archive and keeps cancellation pending until the native result", async () => {
  let status: BackupStatus = {state:"idle"};
  const { backend } = fakeMail({
    backupStatus: vi.fn(async () => status),
    createBackup: vi.fn(async () => {status = running();return 7;}),
    cancelBackup: vi.fn(async () => {status = running(true);return true;}),
  });
  render(<BackupSettings />);
  expect(screen.getByRole("button", {name:"Create recovery archive"})).toBeDisabled();
  const create = await screen.findByRole("button", {name:"Create recovery archive"});
  await waitFor(() => expect(create).toBeEnabled());
  expect(screen.getByText(/Mail kept only on the server/)).toBeInTheDocument();
  const user = userEvent.setup();
  await user.click(create);
  expect(backend.createBackup).toHaveBeenCalledOnce();
  expect(await screen.findByText("Copying local mail…")).toBeInTheDocument();
  expect(screen.getByRole("progressbar")).toHaveAttribute("value", "128");
  expect(create).toBeDisabled();
  await user.click(screen.getByRole("button", {name:"Cancel backup"}));
  expect(backend.cancelBackup).toHaveBeenCalledWith(7);
  expect(await screen.findByText("Cancelling…")).toBeInTheDocument();
  expect(screen.getByRole("button", {name:"Cancel backup"})).toBeDisabled();
});

it("polls sequentially, shows all phases and retains a published result after cancellation", async () => {
  vi.useFakeTimers();
  let status: BackupStatus = running();
  const { backend } = fakeMail({ backupStatus: vi.fn(async () => status), cancelBackup: vi.fn(async () => { status = complete; return false; }) });
  const view = render(<BackupSettings />);
  await act(async () => {});
  expect(screen.getByRole("progressbar")).toHaveAttribute("max", "500");
  for (const [phase, label] of [["preparing","Preparing archive…"],["checking","Checking archive…"],["hashing","Computing checksum…"],["publishing","Saving archive…"]] as const) {
    status = {state:"running",id:7,cancelling:false,progress:{phase}};
    await act(async () => {await vi.advanceTimersByTimeAsync(1000);});
    expect(screen.getByText(label)).toBeInTheDocument();
  }
  await act(async () => {fireEvent.click(screen.getByRole("button", {name:"Cancel backup"}));});
  expect(screen.getByText(/Archive saved/)).toBeInTheDocument();
  expect(screen.getByText(complete.sha256)).toBeInTheDocument();
  expect(screen.getByRole("button", {name:"Create recovery archive"})).toBeEnabled();
  const calls = vi.mocked(backend.backupStatus).mock.calls.length;
  await act(async () => {await vi.advanceTimersByTimeAsync(5000);});
  expect(backend.backupStatus).toHaveBeenCalledTimes(calls);
  view.unmount();
  render(<BackupSettings />);
  await act(async () => {});
  expect(screen.getByText(/Archive saved/)).toBeInTheDocument();
});

it("does not overlap polls and ignores a stale response after an action", async () => {
  vi.useFakeTimers();
  let resolve!: (status: BackupStatus) => void;
  const { backend } = fakeMail({ backupStatus: vi.fn().mockResolvedValueOnce(running()).mockImplementationOnce(() => new Promise<BackupStatus>((r) => {resolve = r;})).mockResolvedValue(complete) });
  render(<BackupSettings />);
  await act(async () => {});
  await act(async () => {await vi.advanceTimersByTimeAsync(1000);});
  await act(async () => {await vi.advanceTimersByTimeAsync(5000);});
  expect(backend.backupStatus).toHaveBeenCalledTimes(2);
  await act(async () => {fireEvent.click(screen.getByRole("button", {name:"Cancel backup"}));});
  expect(screen.getByText(/Archive saved/)).toBeInTheDocument();
  await act(async () => {resolve(running());});
  expect(screen.getByText(/Archive saved/)).toBeInTheDocument();
  expect(screen.queryByRole("button", {name:"Cancel backup"})).not.toBeInTheDocument();
});

it("keeps failed status checks closed to new work and supports checking again", async () => {
  const { backend } = fakeMail({ backupStatus: vi.fn().mockRejectedValueOnce(new Error("private fixture detail")).mockResolvedValue({state:"idle"}) });
  render(<BackupSettings />);
  expect(await screen.findByRole("alert")).toHaveTextContent("Cannot check backup status");
  expect(screen.queryByText(/private fixture/)).not.toBeInTheDocument();
  expect(screen.getByRole("button", {name:"Create recovery archive"})).toBeDisabled();
  await userEvent.click(screen.getByRole("button", {name:"Check backup status"}));
  await waitFor(() => expect(screen.getByRole("button", {name:"Create recovery archive"})).toBeEnabled());
  expect(backend.backupStatus).toHaveBeenCalledTimes(2);
});

it.each([{state:"unknown"}, null, {state:"running",id:7,cancelling:false,progress:{phase:"copying",copiedPages:5,totalPages:1}}])("refuses malformed status %j", async (status) => {
  fakeMail({backupStatus:vi.fn().mockResolvedValue(status)});
  render(<BackupSettings />);
  expect(await screen.findByRole("alert")).toHaveTextContent("Cannot check backup status");
  expect(screen.getByRole("button", {name:"Create recovery archive"})).toBeDisabled();
});

it("shows a cancelled backup without backend text", async () => {
  fakeMail({backupStatus:vi.fn(async () => ({state:"cancelled" as const,id:7,message:"private fixture diagnostic"}))});
  render(<BackupSettings />);
  expect(await screen.findByText(/Backup cancelled/)).toBeInTheDocument();
  expect(screen.queryByText(/private fixture/)).not.toBeInTheDocument();
  expect(screen.getByRole("button", {name:"Create recovery archive"})).toBeEnabled();
});

it("says why a backup failed when the backend says so", async () => {
  const message = "The backup was saved, but reading it back failed, so it can't be trusted. Delete it and make a new one.";
  fakeMail({backupStatus:vi.fn(async () => ({state:"failed" as const,id:7,message}))});
  render(<BackupSettings />);
  expect(await screen.findByText(message)).toBeInTheDocument();
  expect(screen.queryByText(/Archive creation reported a problem/)).not.toBeInTheDocument();
  expect(screen.getByRole("button", {name:"Create recovery archive"})).toBeEnabled();
});

it("shows a fixed text for a failed backup with no message", async () => {
  fakeMail({backupStatus:vi.fn(async () => ({state:"failed" as const,id:7,message:" "}))});
  render(<BackupSettings />);
  expect(await screen.findByText(/Archive creation reported a problem/)).toBeInTheDocument();
});

it("rechecks native state when a start reply fails, without assuming the job failed", async () => {
  const {backend} = fakeMail({backupStatus:vi.fn().mockResolvedValueOnce({state:"idle"}).mockResolvedValue(running()), createBackup:vi.fn().mockRejectedValue(new Error("private failure"))});
  render(<BackupSettings />);
  await waitFor(() => expect(screen.getByRole("button", {name:"Create recovery archive"})).toBeEnabled());
  await userEvent.click(screen.getByRole("button", {name:"Create recovery archive"}));
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not confirm the archive request");
  expect(screen.getByRole("button", {name:"Create recovery archive"})).toBeDisabled();
  expect(await screen.findByRole("button", {name:"Cancel backup"})).toBeEnabled();
  expect(backend.backupStatus).toHaveBeenCalledTimes(2);
});

it("reports cancellation request failure while retaining the running job", async () => {
  fakeMail({backupStatus:vi.fn(async () => running()), cancelBackup:vi.fn().mockRejectedValue(new Error("private failure"))});
  render(<BackupSettings />);
  await userEvent.click(await screen.findByRole("button", {name:"Cancel backup"}));
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not request cancellation");
  expect(screen.getByRole("button", {name:"Create recovery archive"})).toBeDisabled();
});

it("does not cancel the native job or continue polling after unmount", async () => {
  vi.useFakeTimers();
  const {backend} = fakeMail({backupStatus:vi.fn(async () => running())});
  const view = render(<BackupSettings />);
  await act(async () => {});
  view.unmount();
  await act(async () => {await vi.advanceTimersByTimeAsync(3000);});
  expect(backend.backupStatus).toHaveBeenCalledOnce();
  expect(backend.cancelBackup).not.toHaveBeenCalled();
});

it("stops stale timer work while cancellation is awaiting a reply", async () => {
  vi.useFakeTimers();
  let finish!: () => void;
  const {backend} = fakeMail({backupStatus:vi.fn(async () => running()), cancelBackup:vi.fn(() => new Promise<boolean>((resolve) => {finish = () => resolve(true);}))});
  const view = render(<BackupSettings />);
  await act(async () => {});
  act(() => {
    const button = screen.getByRole("button", {name:"Cancel backup"});
    button.click(); button.click();
  });
  expect(backend.cancelBackup).toHaveBeenCalledOnce();
  await act(async () => {await vi.advanceTimersByTimeAsync(2000);});
  expect(backend.backupStatus).toHaveBeenCalledOnce();
  view.unmount();
  await act(async () => {finish();});
  expect(backend.backupStatus).toHaveBeenCalledOnce();
});

it("ignores a rejected action after navigating away", async () => {
  let fail!: () => void;
  const {backend} = fakeMail({createBackup:vi.fn(() => new Promise<number>((_,reject) => {fail = () => reject(new Error("private failure"));}))});
  const view = render(<BackupSettings />);
  await waitFor(() => expect(screen.getByRole("button", {name:"Create recovery archive"})).toBeEnabled());
  await userEvent.click(screen.getByRole("button", {name:"Create recovery archive"}));
  view.unmount();
  await act(async () => {fail();});
  expect(backend.backupStatus).toHaveBeenCalledOnce();
});

it("ignores a failed old poll after a newer cancellation response", async () => {
  vi.useFakeTimers();
  let fail!: () => void;
  fakeMail({backupStatus:vi.fn().mockResolvedValueOnce(running()).mockImplementationOnce(() => new Promise<BackupStatus>((_,reject) => {fail = () => reject(new Error("old failure"));})).mockResolvedValue(complete)});
  render(<BackupSettings />);
  await act(async () => {});
  await act(async () => {await vi.advanceTimersByTimeAsync(1000);});
  await act(async () => {fireEvent.click(screen.getByRole("button", {name:"Cancel backup"}));});
  await act(async () => {fail();});
  expect(screen.getByText(/Archive saved/)).toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it.each([false, true])("drops an old action when the backend changes (failure: %s)", async (fails) => {
  let finish!: () => void;
  fakeMail({createBackup:vi.fn(() => new Promise<number>((resolve,reject) => {finish = () => fails ? reject(new Error("old failure")) : resolve(7);} ))});
  const view = render(<BackupSettings />);
  await waitFor(() => expect(screen.getByRole("button", {name:"Create recovery archive"})).toBeEnabled());
  await userEvent.click(screen.getByRole("button", {name:"Create recovery archive"}));
  const {backend} = fakeMail({backupStatus:vi.fn(async () => complete)});
  view.rerender(<BackupSettings />);
  expect(await screen.findByText(/Archive saved/)).toBeInTheDocument();
  await act(async () => {finish();});
  expect(screen.getByRole("button", {name:"Create recovery archive"})).toBeEnabled();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(backend.backupStatus).toHaveBeenCalledOnce();
});
