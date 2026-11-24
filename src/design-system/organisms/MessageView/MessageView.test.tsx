import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { Message } from "../../../data/types";
import { jonas, maya, message, person, work } from "../../../test/fixtures";

import { MessageView } from "./MessageView";

const email = message({
  to: [maya],
  cc: [jonas],
  time: "9:12 AM",
  paragraphs: ["Hi Maya,", "Notes attached."],
  attachments: [
    { index: 0, name: "notes.pdf", size: "120 KB" },
    { index: 2, name: "budget.xlsx", size: "48 KB" },
  ],
});

function setup(recipientsOpen = false) {
  const onToggleRecipients = vi.fn();
  render(<MessageView message={email} account={work} recipientsOpen={recipientsOpen} onToggleRecipients={onToggleRecipients} />);
  return { onToggleRecipients, user: userEvent.setup() };
}

describe("MessageView", () => {
  it("shows the sender, the time and every paragraph", () => {
    setup();
    const article = screen.getByRole("article");
    expect(article).toHaveTextContent("Lena Vogt");
    expect(article).toHaveTextContent("9:12 AM");
    expect(screen.getByText("Hi Maya,")).toBeInTheDocument();
    expect(screen.getByText("Notes attached.")).toBeInTheDocument();
  });

  it("summarises the recipients, calling you you", () => {
    setup();
    expect(screen.getByRole("button", { name: /to you\s+·\s+cc Jonas/ })).toHaveAttribute("aria-expanded", "false");
  });

  it("toggles the recipient details from the summary", async () => {
    const { onToggleRecipients, user } = setup();
    expect(screen.queryByRole("region", { name: "People on this email" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /to you/ }));
    expect(onToggleRecipients).toHaveBeenCalledOnce();
  });

  it("shows the recipient details when open, and Hide closes them", async () => {
    const { onToggleRecipients, user } = setup(true);
    expect(screen.getByRole("button", { name: /to you/ })).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("region", { name: "People on this email" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Hide/ }));
    expect(onToggleRecipients).toHaveBeenCalledOnce();
  });

  it("lists attachments together, with their kinds and sizes", () => {
    setup();
    const files = within(screen.getByRole("group", { name: "Attachments" }));
    expect(files.getByRole("button", { name: "notes.pdf, PDF document, 120 KB" })).toBeInTheDocument();
    expect(files.getByRole("button", { name: "budget.xlsx, Excel spreadsheet, 48 KB" })).toBeInTheDocument();
  });

  it("shows an attachment's thumbnail and goes by its content type", () => {
    const scan = message({ attachments: [{ index: 0, name: "scan", size: "1 MB", contentType: "application/pdf", preview: "data:,page" }] });
    render(<MessageView message={scan} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} />);
    const file = screen.getByRole("button", { name: "scan, PDF document, 1 MB" });
    expect(file.querySelector("img")).toHaveAttribute("src", "data:,page");
  });

  it("opens an attachment when you click it", async () => {
    const onOpenAttachment = vi.fn();
    render(
      <MessageView message={email} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} onOpenAttachment={onOpenAttachment} />,
    );
    await userEvent.setup().click(screen.getByRole("button", { name: /budget\.xlsx/ }));
    expect(onOpenAttachment).toHaveBeenCalledWith(2);
  });

  it("demo attachments have nothing to open", async () => {
    const onOpenAttachment = vi.fn();
    const demo = message({ attachments: [{ name: "notes.pdf", size: "120 KB" }] });
    render(<MessageView message={demo} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} onOpenAttachment={onOpenAttachment} />);
    await userEvent.setup().click(screen.getByRole("button", { name: /notes\.pdf/ }));
    expect(onOpenAttachment).not.toHaveBeenCalled();
  });

  it("with a sender card, the sender's name opens it", async () => {
    render(
      <MessageView message={email} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} senderCard={<p>Card</p>} />,
    );
    const name = screen.getByRole("button", { name: "Lena Vogt" });
    expect(name).toHaveAttribute("aria-expanded", "false");
    await userEvent.setup().click(name);
    expect(screen.getByRole("dialog", { name: "About Lena Vogt" })).toHaveTextContent("Card");
    expect(name).toHaveAttribute("aria-expanded", "true");
  });

  it("without one, the sender's name is just text", () => {
    setup();
    expect(screen.queryByRole("button", { name: "Lena Vogt" })).not.toBeInTheDocument();
    expect(screen.getByText("Lena Vogt")).toBeInTheDocument();
  });

  it("shows no attachments when there are none", () => {
    render(<MessageView message={message()} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} />);
    expect(screen.getAllByRole("button")).toHaveLength(1);
    expect(screen.queryByRole("group", { name: "Attachments" })).not.toBeInTheDocument();
  });
});

describe("MessageView body", () => {
  it("shows an HTML email in its sandboxed frame, following the theme and reading size", () => {
    const html = message({ html: "<p>Hello from <b>HTML</b></p>" });
    const { rerender } = render(
      <MessageView message={html} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} theme="light" readingSize="larger" />,
    );
    expect((screen.getByTitle("Email") as HTMLIFrameElement).srcdoc).toContain('data-theme="light" data-reading="larger"');
    rerender(<MessageView message={html} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} />);
    expect((screen.getByTitle("Email") as HTMLIFrameElement).srcdoc).toContain('data-theme="dark" data-reading="default"');
  });

  it("passes the image choice and link clicks through", async () => {
    const onShowImages = vi.fn();
    const onOpenLink = vi.fn();
    const remote = message({ text: "See https://northwind.co", html: undefined });
    const { rerender } = render(
      <MessageView message={remote} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} onOpenLink={onOpenLink} />,
    );
    await userEvent.setup().click(screen.getByRole("link", { name: "https://northwind.co" }));
    expect(onOpenLink).toHaveBeenCalledWith("https://northwind.co");

    const onAllowImages = vi.fn();
    const pixel = message({ html: `<p>Hi</p><img src="https://t.example/p.gif" alt="">` });
    const view = (more: object) => (
      <MessageView message={pixel} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} {...more} />
    );
    rerender(view({ onShowImages, onAllowImages }));
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /Just this once/ }));
    expect(onShowImages).toHaveBeenCalledOnce();
    await user.click(screen.getByRole("button", { name: /Always for this sender/ }));
    expect(onAllowImages).toHaveBeenCalledOnce();
    rerender(view({ imagesShown: true }));
    expect(screen.queryByRole("button", { name: /Show images/ })).not.toBeInTheDocument();
  });

  it("shows images by itself for a familiar sender, but not for a new one", () => {
    const html = `<p>Hi</p><img src="https://img.example/hero.jpg" alt="">`;
    const { rerender } = render(
      <MessageView message={message({ html, imageTrust: "you-wrote" })} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} />,
    );
    expect(screen.queryByRole("button", { name: /Show images/ })).not.toBeInTheDocument();
    rerender(
      <MessageView message={message({ html, imageTrust: "ask" })} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} />,
    );
    expect(screen.getByRole("button", { name: /Always for this sender/ })).toBeInTheDocument();
  });

  it("asks first when the mail server didn't confirm a familiar sender sent it, and says why", () => {
    const html = `<p>Hi</p><img src="https://img.example/hero.jpg" alt="">`;
    const view = (m: Partial<Message>) => (
      <MessageView message={message({ html, ...m })} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} />
    );
    const why = "Images are off because Moss couldn't confirm who sent this.";
    const { rerender } = render(view({ imageTrust: "you-wrote", senderUnverified: true }));
    expect((screen.getByTitle("Email") as HTMLIFrameElement).srcdoc).not.toContain("img.example");
    expect(screen.getByText(why)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Always for this sender/ })).not.toBeInTheDocument();
    // A new sender is asked about as before: unconfirmed or not, they aren't allowed yet.
    rerender(view({ imageTrust: "ask", senderUnverified: true }));
    expect(screen.getByText("Images are hidden")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Always for this sender/ })).toBeInTheDocument();
    rerender(view({ imageTrust: "yours", senderUnverified: true }));
    expect(screen.getByText(why)).toBeInTheDocument();
  });

  it("removes trackers that carry your address", () => {
    const html = `<p>Hi</p><img src="https://news.example/a.png?to=${work.address}" alt="">`;
    render(<MessageView message={message({ html, imageTrust: "you-wrote" })} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} />);
    expect((screen.getByTitle("Email") as HTMLIFrameElement).srcdoc).not.toContain("news.example");
  });

  it("opens links in the browser by default", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    render(<MessageView message={message({ text: "https://northwind.co" })} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} />);
    await userEvent.setup().click(screen.getByRole("link"));
    expect(open).toHaveBeenCalledWith("https://northwind.co", "_blank", "noopener,noreferrer");
    open.mockRestore();
  });

  it("does nothing on Show images unless told how", async () => {
    const pixel = message({ html: `<img src="https://t.example/p.gif" alt="">x` });
    render(<MessageView message={pixel} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /Always for this sender/ }));
    await user.click(screen.getByRole("button", { name: /Just this once/ }));
    expect(screen.getByRole("button", { name: /Always for this sender/ })).toBeInTheDocument();
  });

  describe("forwarded", () => {
    const rebecca = person("Rebecca Vale", "rebecca@vale.example", { short: "Rebecca" });
    const forwarded = (extra: Partial<typeof email>) => {
      const onOpenForward = vi.fn();
      render(<MessageView message={{ ...email, ...extra }} account={work} recipientsOpen={false} onToggleRecipients={() => {}} onOpenForward={onOpenForward} />);
      return { onOpenForward, user: userEvent.setup() };
    };

    it("says who you forwarded it to and when, and opens that forward", async () => {
      const { onOpenForward, user } = forwarded({ forwards: [{ threadId: "77", to: [rebecca, jonas], time: "Saturday, 31 October, 4:12 PM" }] });
      const line = screen.getByRole("button", { name: /You forwarded this to Rebecca and Jonas/ });
      expect(line).toHaveTextContent("Saturday, 31 October, 4:12 PM");
      await user.click(line);
      expect(onOpenForward).toHaveBeenCalledWith("77");
    });

    it("lists each forward, using the full name when there is no short one", () => {
      const ines = person("Ines Duarte", "ines@duarte-studio.pt");
      forwarded({
        forwards: [
          { threadId: "77", to: [rebecca], time: "Saturday, 31 October, 4:12 PM" },
          { threadId: "78", to: [ines], time: "Sunday, 1 November, 9:00 AM" },
        ],
        forwarded: true,
      });
      expect(screen.getByRole("button", { name: /to Rebecca\b/ })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /to Ines Duarte/ })).toBeInTheDocument();
    });

    it("says only that it was forwarded when another mail app did it", () => {
      forwarded({ forwarded: true });
      expect(screen.getByText("You forwarded this")).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: /forwarded/ })).not.toBeInTheDocument();
    });

    it("says nothing about an email you never forwarded", () => {
      setup();
      expect(screen.queryByText(/forwarded/)).not.toBeInTheDocument();
    });
  });
});

describe("quoted text", () => {
  const forward = message({ html: `<div>FYI</div><blockquote type="cite">The invoice is due.</blockquote>` });
  const view = (quoteIsNew?: boolean) =>
    render(<MessageView message={forward} account={work} recipientsOpen={false} onToggleRecipients={vi.fn()} quoteIsNew={quoteIsNew} />);

  it("stays open when it is new to you", () => {
    view(true);
    expect((screen.getByTitle("Email") as HTMLIFrameElement).srcdoc).toContain("The invoice is due.");
  });

  it("collapses otherwise", () => {
    view();
    expect((screen.getByTitle("Email") as HTMLIFrameElement).srcdoc).not.toContain("The invoice is due.");
  });
});
