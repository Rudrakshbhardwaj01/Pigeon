import { describe, expect, it } from "vitest";
import { formatBytes, formatDuration, humanPhase } from "./format";

describe("formatBytes", () => {
  it("formats bytes", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
  });
  it("formats kilobytes", () => {
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(100 * 1024)).toBe("100 KB");
  });
  it("formats megabytes", () => {
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
  });
  it("formats gigabytes", () => {
    expect(formatBytes(1024 * 1024 * 1024)).toBe("1.0 GB");
    expect(formatBytes(3.5 * 1024 * 1024 * 1024)).toBe("3.5 GB");
  });
  it("caps at terabytes without breaking", () => {
    expect(formatBytes(2048 * 1024 * 1024 * 1024)).toBe("2.0 TB");
  });
  it("handles null and undefined honestly", () => {
    expect(formatBytes(null)).toBe("—");
    expect(formatBytes(undefined)).toBe("—");
  });
});

describe("formatDuration", () => {
  it("formats milliseconds", () => {
    expect(formatDuration(420)).toBe("420 ms");
  });
  it("formats seconds", () => {
    expect(formatDuration(1500)).toBe("1.5 s");
  });
  it("formats minutes", () => {
    expect(formatDuration(95_000)).toBe("1m 35s");
  });
});

describe("humanPhase", () => {
  it("maps known phases", () => {
    expect(humanPhase("collecting")).toBe("Collecting packets");
    expect(humanPhase("verified")).toBe("Verified");
  });
  it("passes unknown phases through", () => {
    expect(humanPhase("mystery")).toBe("mystery");
  });
});
