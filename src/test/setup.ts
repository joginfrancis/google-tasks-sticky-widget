import "@testing-library/jest-dom/vitest";
import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";

// Every test gets a clean DOM and fresh mock call counts, so assertions like
// "onSubmit was called once" mean once *in this test*.
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
