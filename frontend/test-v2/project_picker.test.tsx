import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ProjectPicker } from "../src/ui/ProjectPicker.js";

const options = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"].map((name) => ({
  value: name,
  label: name,
  detail: `/fixture/${name}`,
}));

describe("ProjectPicker", () => {
  it("searches projects, changes the exact value, and restores focus", () => {
    const onChange = vi.fn();
    render(<ProjectPicker label="Projects" allLabel="All Projects" emptyLabel="No projects" searchLabel="Search projects" options={options} value="" onChange={onChange} />);

    const trigger = screen.getByRole("button", { name: "Projects: All Projects" });
    fireEvent.click(trigger);
    expect(screen.getByRole("dialog", { name: "Projects" })).toBeTruthy();
    const search = screen.getByRole("textbox", { name: "Search projects" });
    expect(document.activeElement).toBe(search);
    fireEvent.change(search, { target: { value: "beta" } });
    const menu = within(screen.getByRole("dialog", { name: "Projects" }));
    expect(menu.getByRole("button", { name: /beta/ })).toBeTruthy();
    expect(menu.queryByRole("button", { name: /alpha/ })).toBeNull();
    fireEvent.click(menu.getByRole("button", { name: /beta/ }));
    expect(onChange).toHaveBeenCalledExactlyOnceWith("beta");
    expect(screen.queryByRole("dialog", { name: "Projects" })).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("supports arrow keys, Escape, and choosing all projects", () => {
    const onChange = vi.fn();
    render(<ProjectPicker label="Projects" allLabel="All Projects" emptyLabel="No projects" searchLabel="Search projects" options={options.slice(0, 2)} value="alpha" onChange={onChange} />);

    const trigger = screen.getByRole("button", { name: "Projects: alpha" });
    fireEvent.click(trigger);
    const menu = within(screen.getByRole("dialog", { name: "Projects" }));
    expect(document.activeElement).toBe(menu.getByRole("button", { name: /alpha/ }));
    fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    expect(document.activeElement).toBe(menu.getByRole("button", { name: /beta/ }));
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Projects" })).toBeNull();
    expect(document.activeElement).toBe(trigger);

    fireEvent.click(trigger);
    fireEvent.click(within(screen.getByRole("dialog", { name: "Projects" })).getByRole("button", { name: "All Projects" }));
    expect(onChange).toHaveBeenCalledExactlyOnceWith("");
  });

  it("clears a dismissed search before reopening", () => {
    render(<ProjectPicker label="Projects" allLabel="All Projects" emptyLabel="No projects" searchLabel="Search projects" options={options} value="" onChange={vi.fn()} />);
    const trigger = screen.getByRole("button", { name: "Projects: All Projects" });
    fireEvent.click(trigger);
    fireEvent.change(screen.getByRole("textbox", { name: "Search projects" }), { target: { value: "unlisted" } });
    expect(screen.getByText("No projects")).toBeTruthy();
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Search projects" }), { key: "Escape" });
    fireEvent.click(trigger);
    expect((screen.getByRole("textbox", { name: "Search projects" }) as HTMLInputElement).value).toBe("");
    expect(within(screen.getByRole("dialog", { name: "Projects" })).getByRole("button", { name: /alpha/ })).toBeTruthy();
  });
});


it("closes an open resource picker when its owning operation becomes disabled", () => {
  const onChange = vi.fn();
  const props = { label: "Projects", allLabel: "All Projects", emptyLabel: "No projects", searchLabel: "Search projects", options, value: "alpha", onChange };
  const view = render(<ProjectPicker {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "Projects: alpha" }));
  expect(screen.getByRole("dialog", { name: "Projects" })).toBeTruthy();
  view.rerender(<ProjectPicker {...props} disabled />);
  expect(screen.queryByRole("dialog", { name: "Projects" })).toBeNull();
  expect(onChange).not.toHaveBeenCalled();
  view.rerender(<ProjectPicker {...props} />);
  expect(screen.queryByRole("dialog", { name: "Projects" })).toBeNull();
});
