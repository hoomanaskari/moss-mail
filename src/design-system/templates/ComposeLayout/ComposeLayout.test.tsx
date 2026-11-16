import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ComposeLayout } from "./ComposeLayout";

describe("ComposeLayout", () => {
  it("puts the content first and pins the footer last, with a spacer between", () => {
    const { container } = render(
      <ComposeLayout footer={<div>Toolbar</div>}>
        <div>Fields</div>
        <div>Editor</div>
      </ComposeLayout>,
    );
    const column = container.firstElementChild!;
    expect([...column.children].map((c) => c.textContent)).toEqual(["Fields", "Editor", "", "Toolbar"]);
  });
});
