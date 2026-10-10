import { describe, expect, test, vi } from "vitest";
import { ApiError, AuthError } from "../api";
import { failure } from "./errors";

describe("failure", () => {
  test("a 401 sends the user to sign-in", () => {
    const onAuthLost = vi.fn();
    const text = failure(new AuthError(), onAuthLost);
    expect(onAuthLost).toHaveBeenCalledOnce();
    expect(text).not.toMatch(/Error/);
  });

  test("other errors are shown and do not sign anyone out", () => {
    const onAuthLost = vi.fn();
    expect(failure(new ApiError(400, "bad pattern"), onAuthLost)).toBe("bad pattern");
    expect(failure(new TypeError("network"), onAuthLost)).toMatch(/network/);
    expect(onAuthLost).not.toHaveBeenCalled();
  });
});
