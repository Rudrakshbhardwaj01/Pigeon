import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Dropzone } from "./Dropzone";
import { ErrorDisplay } from "./ErrorDisplay";
import { ProgressBar, Stat } from "./Stats";

describe("Dropzone", () => {
  it("renders label and hint", () => {
    render(
      <Dropzone label="Drop a file here" hint="any type" onFile={() => {}} />
    );
    expect(screen.getByRole("button", { name: "Drop a file here" })).toBeVisible();
    expect(screen.getByText("any type")).toBeVisible();
  });

  it("opens the file chooser on click", async () => {
    const user = userEvent.setup();
    render(<Dropzone label="Drop" hint="h" onFile={() => {}} />);
    const input = document.querySelector('input[type="file"]') as HTMLInputElement;
    const clickSpy = vi.spyOn(input, "click");
    await user.click(screen.getByRole("button", { name: "Drop" }));
    expect(clickSpy).toHaveBeenCalled();
  });

  it("rejects non-GIF files when gifOnly is set", () => {
    const onFile = vi.fn();
    const onRejected = vi.fn();
    render(
      <Dropzone label="Drop" hint="h" onFile={onFile} gifOnly onRejected={onRejected} />
    );
    const input = document.querySelector('input[type="file"]') as HTMLInputElement;
    const file = new File(["x"], "notes.txt", { type: "text/plain" });
    fireEvent.change(input, { target: { files: [file] } });
    expect(onFile).not.toHaveBeenCalled();
    expect(onRejected).toHaveBeenCalledOnce();
  });

  it("accepts GIF files when gifOnly is set", () => {
    const onFile = vi.fn();
    render(<Dropzone label="Drop" hint="h" onFile={onFile} gifOnly />);
    const input = document.querySelector('input[type="file"]') as HTMLInputElement;
    const file = new File(["x"], "transfer.gif", { type: "image/gif" });
    fireEvent.change(input, { target: { files: [file] } });
    expect(onFile).toHaveBeenCalledOnce();
  });

  it("is keyboard accessible", async () => {
    const user = userEvent.setup();
    render(<Dropzone label="Drop" hint="h" onFile={() => {}} />);
    const zone = screen.getByRole("button", { name: "Drop" });
    await user.tab();
    expect(zone).toHaveFocus();
    const input = document.querySelector('input[type="file"]') as HTMLInputElement;
    const clickSpy = vi.spyOn(input, "click");
    await user.keyboard("{Enter}");
    expect(clickSpy).toHaveBeenCalled();
  });
});

describe("ErrorDisplay", () => {
  it("renders error tone with role=alert", () => {
    render(<ErrorDisplay title="Something failed" detail="more detail" />);
    expect(screen.getByRole("alert")).toBeVisible();
    expect(screen.getByText("Something failed")).toBeVisible();
    expect(screen.getByText("more detail")).toBeVisible();
  });

  it("renders success tone with role=status", () => {
    render(<ErrorDisplay title="All good" tone="success" />);
    expect(screen.getByRole("status")).toBeVisible();
  });
});

describe("ProgressBar", () => {
  it("renders a determinate value with aria attributes", () => {
    render(<ProgressBar value={0.42} label="Collecting packets" />);
    const bar = screen.getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuenow", "42");
    expect(screen.getByText("42%")).toBeVisible();
  });

  it("renders indeterminate state as em-dash", () => {
    render(<ProgressBar value={null} label="Preparing" />);
    expect(screen.getByText("—")).toBeVisible();
    expect(screen.getByRole("progressbar")).not.toHaveAttribute("aria-valuenow");
  });
});

describe("Stat", () => {
  it("renders label and value", () => {
    render(<Stat label="Frames" value="1234" mono />);
    expect(screen.getByText("Frames")).toBeVisible();
    expect(screen.getByText("1234")).toBeVisible();
  });
});
