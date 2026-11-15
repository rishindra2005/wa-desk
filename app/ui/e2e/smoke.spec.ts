import { test, expect } from "@playwright/test";

test("smoke: app boots with no console errors", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.goto("/?quiet");
  await expect(page.getByTestId("chat-row").first()).toBeVisible();
  await page.getByTestId("chat-row").first().click();
  await expect(page.getByTestId("message").first()).toBeVisible();
  await page.screenshot({ path: "test-results/smoke.png" });
  expect(errors).toEqual([]);
});
