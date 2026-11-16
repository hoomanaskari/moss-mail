import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { Markdown } from "./Markdown";

const openLink = vi.hoisted(() => vi.fn());
vi.mock("../../../lib/links", () => ({ openLink }));

beforeEach(() => openLink.mockClear());

it("renders what a model writes: emphasis, headings, lists, quotes, code and tables", () => {
  const text = [
    "## What matters",
    "",
    "The email is from **The Tinker Weekly**, *issue 28*, with `code`.",
    "",
    "- First point",
    "- Second point",
    "  - Nested",
    "",
    "1. One",
    "2. Two",
    "",
    "> Quoted",
    "",
    "```",
    "let x = 1;",
    "```",
    "",
    "| Who | Owes |",
    "| --- | --- |",
    "| Dana | 182k |",
    "",
    "~~gone~~",
  ].join("\n");
  const { container } = render(<Markdown text={text} />);
  expect(screen.getByRole("heading", { name: "What matters" })).toBeInTheDocument();
  expect(screen.getByText("The Tinker Weekly").tagName).toBe("STRONG");
  expect(screen.getByText("issue 28").tagName).toBe("EM");
  expect(screen.getByText("code").tagName).toBe("CODE");
  const lists = screen.getAllByRole("list");
  expect(within(lists[0]).getAllByRole("listitem").map((li) => li.firstChild?.textContent?.trim())).toEqual(["First point", "Second point", "Nested"]);
  expect(container.querySelector("ol")).toHaveTextContent("One Two");
  expect(container.querySelector("blockquote")).toHaveTextContent("Quoted");
  expect(container.querySelector("pre")).toHaveTextContent("let x = 1;");
  expect(screen.getByRole("columnheader", { name: "Owes" })).toBeInTheDocument();
  expect(screen.getByRole("cell", { name: "182k" })).toBeInTheDocument();
  expect(screen.getByText("gone").tagName).toBe("DEL");
  expect(container).not.toHaveTextContent("**");
  expect(container).not.toHaveTextContent("##");
});

it("keeps the model's line breaks, and a <br> it writes is a line break", () => {
  const { container } = render(<Markdown text={"Line one\nLine two<br>Line three"} />);
  expect(container.querySelectorAll("br")).toHaveLength(2);
  expect(container).not.toHaveTextContent("<br>");
});

it("never runs what a model or an email slipped in", () => {
  const { container } = render(<Markdown text={'Hi <script>alert(1)</script><img src=x onerror="alert(2)"> [x](javascript:alert(3)) <iframe src="https://x.co"></iframe>'} />);
  expect(container.querySelector("script, iframe, img")).toBeNull();
  expect(container.innerHTML).not.toMatch(/onerror|javascript:/);
});

it("opens links outside Moss, and only web and mail links", () => {
  render(<Markdown text={"See [the site](https://example.com/a) or <https://moss.example>, or write to <hi@moss.example>."} />);
  const link = screen.getByRole("link", { name: "the site" });
  fireEvent.click(link);
  expect(openLink).toHaveBeenCalledWith("https://example.com/a");
  fireEvent.click(screen.getByRole("link", { name: "hi@moss.example" }));
  expect(openLink).toHaveBeenLastCalledWith("mailto:hi@moss.example");
  // A click beside a link opens nothing.
  fireEvent.click(screen.getByText(/See/));
  expect(openLink).toHaveBeenCalledTimes(2);
  // A link whose address was taken out goes nowhere.
  render(<Markdown text="[run](javascript:alert(1))" />);
  fireEvent.click(screen.getByText("run"));
  expect(openLink).toHaveBeenLastCalledWith("");
});

it("inline, it stays in its line: emphasis and code, no paragraphs or lists", () => {
  const { container } = render(<Markdown text={"Send the **Q4** numbers by `Friday`"} inline />);
  expect(container.firstElementChild?.tagName).toBe("SPAN");
  expect(container.querySelector("p, ul")).toBeNull();
  expect(screen.getByText("Q4").tagName).toBe("STRONG");
});
