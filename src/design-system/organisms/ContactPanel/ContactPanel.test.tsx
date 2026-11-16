import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { lena, person } from "../../../test/fixtures";
import { ContactPanel } from "./ContactPanel";

const ines = person("Ines Duarte", "ines@studio-ines.pt", { role: "Designer", location: "Lisbon", localTime: "8:12 AM" });

describe("ContactPanel", () => {
  it("shows their name, initials and email", () => {
    const { container } = render(<ContactPanel person={lena} />);
    expect(container).toHaveTextContent("Lena Vogt");
    expect(container).toHaveTextContent("LV");
    expect(container).toHaveTextContent("lena@northwind.co");
  });

  it("shows the address once when it is all the name there is", () => {
    render(<ContactPanel person={person("noreply@power-utility.example", "noreply@power-utility.example")} />);
    expect(screen.getAllByText("noreply@power-utility.example")).toHaveLength(1);
  });

  it("leaves out role, place, recent emails and insight when there are none", () => {
    render(<ContactPanel person={lena} />);
    expect(screen.queryByRole("heading", { name: "Recent" })).not.toBeInTheDocument();
    expect(screen.queryByText("Good to know")).not.toBeInTheDocument();
    expect(screen.queryByText(/there$/)).not.toBeInTheDocument();
  });

  it("shows role, place and their local time", () => {
    render(<ContactPanel person={ines} />);
    expect(screen.getByText("Designer")).toBeInTheDocument();
    expect(screen.getByText("Lisbon · 8:12 AM there", { normalizer: (s) => s.replace(/\s+/g, " ") })).toBeInTheDocument();
  });

  it("shows the place alone when the local time is unknown", () => {
    render(<ContactPanel person={{ ...ines, localTime: undefined }} />);
    expect(screen.getByText("Lisbon")).toBeInTheDocument();
    expect(screen.queryByText(/there/)).not.toBeInTheDocument();
  });

  it("lists recent emails under a heading", () => {
    render(
      <ContactPanel
        person={lena}
        recent={[
          { subject: "Q4 planning", when: "Tue" },
          { subject: "Offsite", when: "Oct 12" },
        ]}
      />,
    );
    expect(screen.getByRole("heading", { name: "Recent" })).toBeInTheDocument();
    expect(screen.getByText("Q4 planning")).toBeInTheDocument();
    expect(screen.getByText("Tue")).toBeInTheDocument();
    expect(screen.getByText("Offsite")).toBeInTheDocument();
    expect(screen.getByText("Oct 12")).toBeInTheDocument();
  });

  it("shows what the AI noticed", () => {
    render(<ContactPanel person={lena} insight="Usually replies within an hour." />);
    expect(screen.getByText("Good to know")).toBeInTheDocument();
    expect(screen.getByText("Usually replies within an hour.")).toBeInTheDocument();
  });

  it("shows what you have in common with them", () => {
    render(<ContactPanel person={lena} between={{ emails: 6, since: "March", youReply: "Same day" }} />);
    expect(screen.getByRole("heading", { name: "Between you" })).toBeInTheDocument();
    expect(screen.getByText("Emails together").nextSibling).toHaveTextContent("6");
    expect(screen.getByText("First email").nextSibling).toHaveTextContent("March");
    expect(screen.getByText("You usually reply").nextSibling).toHaveTextContent("Same day");
  });

  it("for a new sender, says whether you have ever written to them", () => {
    render(<ContactPanel person={lena} between={{ emails: 1, since: "Today", youWrote: "Never" }} />);
    expect(screen.getByText("You’ve written to them").nextSibling).toHaveTextContent("Never");
    expect(screen.queryByText("You usually reply")).not.toBeInTheDocument();
  });

  it("leaves out Between you when there is nothing to say", () => {
    render(<ContactPanel person={lena} />);
    expect(screen.queryByRole("heading", { name: "Between you" })).not.toBeInTheDocument();
  });
});
