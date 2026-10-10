import { expect, test } from "vitest";
import { ColorBook, NEUTRAL, PALETTE } from "./colors";

test("a key keeps its color; the ninth and 'other' are neutral", () => {
  const book = new ColorBook();
  const keys = ["a", "b", "c", "d", "e", "f", "g", "h"];
  expect(keys.map((k) => book.get(k))).toEqual(PALETTE);
  expect(book.get("a")).toBe(PALETTE[0]);
  expect(book.get("i")).toBe(NEUTRAL);
  expect(book.get("~other")).toBe(NEUTRAL);
  expect(book.get("")).toBe(NEUTRAL);
});
