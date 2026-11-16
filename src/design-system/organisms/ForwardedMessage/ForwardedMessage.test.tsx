import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { jonas, lena, maya, message, person } from "../../../test/fixtures";
import { ForwardedMessage, type ForwardedMessageProps } from "./ForwardedMessage";

const images: Pick<ForwardedMessageProps, "onShowImages" | "onAllowImages"> = { onShowImages: () => {}, onAllowImages: () => {} };

describe("ForwardedMessage", () => {
  it("shows who wrote the forwarded email, when, to whom, and what it says", () => {
    render(
      <ForwardedMessage
        {...images}
        message={message({ from: lena, to: [maya], cc: [jonas], paragraphs: ["Your refund is on its way."] })}
        subject="Your refund summary"
      />,
    );
    const quote = screen.getByRole("region", { name: "Forwarded message" });
    expect(within(quote).getByText("Forwarded message")).toBeInTheDocument();
    expect(within(quote).getByText(`${lena.name} <${lena.email}>`)).toBeInTheDocument();
    expect(within(quote).getByText("Sunday, 1 November, 9:12 AM")).toBeInTheDocument();
    expect(within(quote).getByText("Your refund summary")).toBeInTheDocument();
    expect(within(quote).getByText(`${maya.name} <${maya.email}>`)).toBeInTheDocument();
    expect(within(quote).getByText(`${jonas.name} <${jonas.email}>`)).toBeInTheDocument();
    expect(within(quote).getByText("Your refund is on its way.")).toBeInTheDocument();
  });

  it("shows a forwarded HTML email at the reading size", () => {
    const { rerender } = render(<ForwardedMessage {...images} message={message({ html: "<p>Hi</p>" })} subject="Hi" />);
    expect((screen.getByTitle("Email") as HTMLIFrameElement).srcdoc).toContain('data-reading="default"');
    rerender(<ForwardedMessage {...images} message={message({ html: "<p>Hi</p>" })} subject="Hi" readingSize="smaller" />);
    expect((screen.getByTitle("Email") as HTMLIFrameElement).srcdoc).toContain('data-reading="smaller"');
  });

  it("asks first when the mail server didn't confirm a familiar sender sent it", () => {
    const html = `<p>Hi</p><img src="https://img.example/hero.jpg" alt="">`;
    const { rerender } = render(<ForwardedMessage {...images} message={message({ html, imageTrust: "you-wrote" })} subject="Hi" />);
    const frame = () => (screen.getByTitle("Email") as HTMLIFrameElement).srcdoc;
    expect(frame()).toContain("https://img.example/hero.jpg");
    rerender(<ForwardedMessage {...images} message={message({ html, imageTrust: "you-wrote", senderUnverified: true })} subject="Hi" />);
    expect(frame()).not.toContain("img.example");
    expect(screen.getByText("Images are off because Moss couldn't confirm who sent this.")).toBeInTheDocument();
  });

  it("leaves out CC when there was none", () => {
    render(<ForwardedMessage {...images} message={message({ cc: [] })} subject="Hi" />);
    expect(screen.queryByText("CC")).not.toBeInTheDocument();
  });

  it("shows a bare address when the sender has no name", () => {
    const bare = person("", "noreply@shop.example");
    render(<ForwardedMessage {...images} message={message({ from: bare })} subject="Hi" />);
    expect(screen.getByText("noreply@shop.example")).toBeInTheDocument();
  });

  it("opens links in the forwarded text through onOpenLink", async () => {
    const onOpenLink = vi.fn();
    render(<ForwardedMessage {...images} message={message({ paragraphs: ["See https://shop.example/r/1"] })} subject="Hi" onOpenLink={onOpenLink} />);
    await userEvent.setup().click(screen.getByRole("link", { name: "https://shop.example/r/1" }));
    expect(onOpenLink).toHaveBeenCalledWith("https://shop.example/r/1");
  });
});
