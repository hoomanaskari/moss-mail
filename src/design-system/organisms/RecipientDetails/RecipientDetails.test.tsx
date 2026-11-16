import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { jonas, lena, maya, message, person, personal, work } from "../../../test/fixtures";
import { RecipientDetails } from "./RecipientDetails";

const northwind = { ...work, org: "Northwind" };
const ines = person("Ines Duarte", "ines@studio.pt", { short: "Ines" });
const tom = person("Tom Hale", "tom@hale.io");

const group = (label: string) => screen.getByText(label, { selector: "span" }).parentElement!;

describe("RecipientDetails", () => {
  it("counts the sender and every recipient in the title", () => {
    render(
      <RecipientDetails message={message({ to: [maya], cc: [jonas], bcc: [ines] })} account={northwind} onHide={() => {}} />,
    );
    expect(screen.getByRole("region", { name: "People on this email" })).toHaveTextContent("4 people on this email");
  });

  it("only shows the role groups that have people in them", () => {
    render(<RecipientDetails message={message({ to: [maya] })} account={northwind} onHide={() => {}} />);
    expect(screen.getByText("From")).toBeInTheDocument();
    expect(screen.getByText("To")).toBeInTheDocument();
    expect(screen.queryByText("CC")).not.toBeInTheDocument();
    expect(screen.queryByText("BCC")).not.toBeInTheDocument();
  });

  it("lists people under their role, with CC and BCC when present", () => {
    render(
      <RecipientDetails message={message({ to: [maya], cc: [jonas], bcc: [tom] })} account={northwind} onHide={() => {}} />,
    );
    expect(within(group("From")).getByText("Lena Vogt")).toBeInTheDocument();
    expect(within(group("CC")).getByText("Jonas Weber")).toBeInTheDocument();
    expect(within(group("BCC")).getByText("tom@hale.io")).toBeInTheDocument();
  });

  it("shows the account's own address as You with its badge", () => {
    render(<RecipientDetails message={message({ to: [maya] })} account={northwind} onHide={() => {}} />);
    const to = group("To");
    expect(within(to).getByText("You")).toBeInTheDocument();
    expect(within(to).getByText(maya.email)).toBeInTheDocument();
    expect(within(to).getByText("W")).toBeInTheDocument();
    expect(within(to).queryByText("Maya Ross")).not.toBeInTheDocument();
    // Lena is not you: no badge in her line.
    expect(within(group("From")).queryByText("W")).not.toBeInTheDocument();
  });

  it("flags people outside the organisation", () => {
    render(<RecipientDetails message={message({ to: [maya, ines] })} account={northwind} onHide={() => {}} />);
    expect(screen.getAllByText("Outside Northwind")).toHaveLength(1);
  });

  it("shows when the email was sent", () => {
    render(<RecipientDetails message={message({ sentFull: "Monday, 2 November, 4:00 PM" })} account={northwind} onHide={() => {}} />);
    expect(within(group("Sent")).getByText("Monday, 2 November, 4:00 PM")).toBeInTheDocument();
    expect(screen.getByText("your time")).toBeInTheDocument();
  });

  it("calls onHide from the Hide shortcut", async () => {
    const user = userEvent.setup();
    const onHide = vi.fn();
    render(<RecipientDetails message={message()} account={northwind} onHide={onHide} />);
    await user.click(screen.getByRole("button", { name: /Hide/ }));
    expect(onHide).toHaveBeenCalledOnce();
  });

  it("copies a person's address", async () => {
    const user = userEvent.setup();
    render(<RecipientDetails message={message({ to: [maya] })} account={northwind} onHide={() => {}} />);
    await user.click(within(group("From")).getByRole("button", { name: "Copy" }));
    expect(await navigator.clipboard.readText()).toBe(lena.email);
  });

  it("does nothing on copy when there is no clipboard", async () => {
    const user = userEvent.setup();
    const original = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true });
    try {
      render(<RecipientDetails message={message()} account={northwind} onHide={() => {}} />);
      await user.click(within(group("From")).getByRole("button", { name: "Copy" }));
      expect(screen.getByRole("region")).toBeInTheDocument();
    } finally {
      if (original) Object.defineProperty(navigator, "clipboard", original);
    }
  });

  describe("reply all warning", () => {
    it("names one outsider with 'is', using their short name", () => {
      render(<RecipientDetails message={message({ to: [maya, ines] })} account={northwind} onHide={() => {}} />);
      expect(screen.getByText("Reply all includes Ines, who is outside Northwind.")).toBeInTheDocument();
    });

    it("names several outsiders from To and CC with 'are', falling back to the full name", () => {
      render(<RecipientDetails message={message({ to: [ines], cc: [tom] })} account={northwind} onHide={() => {}} />);
      expect(screen.getByText("Reply all includes Ines and Tom Hale, who are outside Northwind.")).toBeInTheDocument();
    });

    it("ignores outsiders who were only BCC'd or who sent it", () => {
      render(
        <RecipientDetails message={message({ from: ines, to: [maya], bcc: [tom] })} account={northwind} onHide={() => {}} />,
      );
      expect(screen.queryByText(/Reply all includes/)).not.toBeInTheDocument();
    });

    it("never warns on an account with no organisation", () => {
      render(<RecipientDetails message={message({ to: [ines] })} account={personal} onHide={() => {}} />);
      expect(screen.queryByText(/Reply all includes/)).not.toBeInTheDocument();
      expect(screen.queryByText(/Outside/)).not.toBeInTheDocument();
    });
  });
});
