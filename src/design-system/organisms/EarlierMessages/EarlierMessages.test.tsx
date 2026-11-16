import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { lena, maya, message } from "../../../test/fixtures";
import { EarlierMessages } from "./EarlierMessages";

const renderMessage = (m: { id: string }) => <p>Full message {m.id}</p>;
const rows = () => screen.queryAllByRole("button", { name: /^(Lena Vogt|You)/ });

describe("EarlierMessages", () => {
  it("lists each earlier message: who sent it, the start of what they said and when", () => {
    render(
      <EarlierMessages
        messages={[
          message({ id: "1", from: lena, paragraphs: ["Hi Maya,", "Are you  free\nnext week?"], time: "Oct 27" }),
          message({ id: "2", from: { ...maya, email: "MAYA@northwind.co" }, paragraphs: ["Yes, Friday."], time: "Oct 28" }),
        ]}
        youAddress={maya.email}
        renderMessage={renderMessage}
      />,
    );
    expect(rows().map((r) => r.textContent)).toEqual(["Lena VogtHi Maya, Are you free next week?Oct 27", "YouYes, Friday.Oct 28"]);
  });

  it("uses the plain text when a message has no paragraphs", () => {
    render(
      <EarlierMessages
        messages={[message({ id: "1", paragraphs: [], text: "Line one\n\nLine two" })]}
        youAddress={maya.email}
        renderMessage={renderMessage}
      />,
    );
    expect(screen.getByRole("button", { name: /Line one Line two/ })).toBeInTheDocument();
  });

  it("shows just who and when for a message with no text at all", () => {
    render(<EarlierMessages messages={[message({ id: "1", paragraphs: [], time: "Oct 2" })]} youAddress={maya.email} renderMessage={renderMessage} />);
    expect(rows()[0].textContent).toBe("Lena VogtOct 2");
  });

  it("opens a message in full when its row is clicked", async () => {
    render(<EarlierMessages messages={[message({ id: "1" }), message({ id: "2" })]} youAddress={maya.email} renderMessage={renderMessage} />);
    await userEvent.setup().click(rows()[1]);
    expect(screen.getByText("Full message 2")).toBeInTheDocument();
    expect(screen.queryByText("Full message 1")).not.toBeInTheDocument();
    expect(rows()).toHaveLength(1);
  });

  it("folds the middle of a long conversation into one button, and shows it all when asked", async () => {
    const messages = [1, 2, 3, 4, 5, 6].map((n) => message({ id: `${n}`, paragraphs: [`Note ${n}`] }));
    render(<EarlierMessages messages={messages} youAddress={maya.email} renderMessage={renderMessage} />);
    expect(rows().map((r) => r.textContent)).toEqual([
      expect.stringContaining("Note 1"),
      expect.stringContaining("Note 2"),
      expect.stringContaining("Note 6"),
    ]);
    await userEvent.setup().click(screen.getByRole("button", { name: "3 more messages" }));
    expect(rows()).toHaveLength(6);
    expect(screen.queryByRole("button", { name: /more messages/ })).not.toBeInTheDocument();
  });

  it("opens a message Ask Moss cited, unfolding the conversation to it", () => {
    const messages = [1, 2, 3, 4, 5, 6].map((n) => message({ id: `${n}`, paragraphs: [`Note ${n}`] }));
    const { container, rerender } = render(<EarlierMessages messages={messages} youAddress={maya.email} renderMessage={renderMessage} reveal="4" />);
    expect(screen.getByText("Full message 4")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /more messages/ })).not.toBeInTheDocument();
    expect(container.querySelector('[data-message="4"]')).toHaveTextContent("Full message 4");
    // One it shows anyway opens without unfolding the rest.
    rerender(<EarlierMessages messages={messages} youAddress={maya.email} renderMessage={renderMessage} reveal="1" />);
    expect(screen.getByText("Full message 1")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "3 more messages" })).toBeInTheDocument();
  });

  it("shows every row when there are only a few", () => {
    const messages = [1, 2, 3, 4].map((n) => message({ id: `${n}` }));
    render(<EarlierMessages messages={messages} youAddress={maya.email} renderMessage={renderMessage} />);
    expect(rows()).toHaveLength(4);
  });

  it("draws nothing without earlier messages", () => {
    const { container } = render(<EarlierMessages messages={[]} youAddress={maya.email} renderMessage={renderMessage} />);
    expect(container).toBeEmptyDOMElement();
  });
});
