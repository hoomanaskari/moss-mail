import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { SenderBanner } from "./SenderBanner";

const handlers = () => ({ onBlock: vi.fn(), onBlockDomain: vi.fn(), onAccept: vi.fn(), onAcceptDomain: vi.fn(), onUnblock: vi.fn() });

describe("SenderBanner", () => {
  it("asks about a new sender, with Block, Accept and the whole domain", async () => {
    const on = handlers();
    render(<SenderBanner status="new" email="jordan@growthleads.example" domain="growthleads.example" account="Studio" {...on} />);
    const banner = screen.getByRole("region", { name: "Block this sender?" });
    expect(banner).toHaveTextContent("From jordan@growthleads.example. You’ve never written to them.");
    await userEvent.click(screen.getByRole("button", { name: "BlockB" }));
    await userEvent.click(screen.getByRole("button", { name: "AcceptA" }));
    const everyone = within(banner).getByRole("group", { name: "Everyone at growthleads.example" });
    await userEvent.click(within(everyone).getByRole("button", { name: "Shift+BBlock" }));
    await userEvent.click(within(everyone).getByRole("button", { name: "Shift+AAccept" }));
    expect(on.onBlock).toHaveBeenCalledOnce();
    expect(on.onAccept).toHaveBeenCalledOnce();
    expect(on.onBlockDomain).toHaveBeenCalledOnce();
    expect(on.onAcceptDomain).toHaveBeenCalledOnce();
    expect(screen.queryByRole("button", { name: /Unblock/ })).not.toBeInTheDocument();
  });

  it("never offers to block or accept a public domain like gmail.com", () => {
    render(<SenderBanner status="new" email="pat@gmail.com" account="Studio" {...handlers()} />);
    expect(screen.queryByRole("group", { name: /Everyone/ })).not.toBeInTheDocument();
    expect(screen.getAllByRole("button").map((b) => b.textContent)).toEqual(["BlockB", "AcceptA"]);
  });

  it("on blocked mail, says where it goes and offers Unblock", async () => {
    const on = handlers();
    render(<SenderBanner status="blocked" email="jordan@growthleads.example" domain="growthleads.example" account="Studio" {...on} />);
    const banner = screen.getByRole("region", { name: "You blocked this sender" });
    expect(banner).toHaveTextContent("Their mail goes to Studio’s Blocked folder and never reaches your inbox.");
    await userEvent.click(screen.getByRole("button", { name: "UnblockU" }));
    expect(on.onUnblock).toHaveBeenCalledOnce();
    expect(screen.queryByRole("button", { name: "BlockB" })).not.toBeInTheDocument();
  });

  it("on mail blocked for the address it came to, says so", () => {
    render(<SenderBanner status="blocked" email="deals@shop.example" blockedAddress="shop@mayaross.me" account="iCloud" {...handlers()} />);
    const banner = screen.getByRole("region", { name: "You blocked mail to shop@mayaross.me" });
    expect(banner).toHaveTextContent("Everything sent to it goes to iCloud’s Blocked folder, whoever it’s from.");
  });
});
