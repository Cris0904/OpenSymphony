import { test, expect, type Page } from '@playwright/test';
import * as fs from 'fs';
import * as path from 'path';

const SCREENSHOT_DIR = path.resolve(process.cwd(), 'e2e-screenshots');

test.beforeAll(() => {
  if (!fs.existsSync(SCREENSHOT_DIR)) {
    fs.mkdirSync(SCREENSHOT_DIR, { recursive: true });
  }
});

// Helper to wait for React app to be mounted and rendering
async function waitForApp(page: Page) {
  await page.goto('/#/dashboard');
  await page.waitForLoadState('networkidle');
  await page.waitForTimeout(1000);
}

test.describe('COE-402 UI Evidence Capture', () => {
  test('capture dashboard page', async ({ page }) => {
    await waitForApp(page);
    // Navigate to dashboard via hash
    await page.evaluate(() => { window.location.hash = '#/dashboard'; });
    await page.waitForTimeout(500);
    // Take screenshot regardless of text matching since fixture data should render
    await page.screenshot({ path: path.join(SCREENSHOT_DIR, 'dashboard.png'), fullPage: false });
    // Verify core dashboard sections render
    const bodyText = await page.textContent('body');
    expect(bodyText).toContain('Dashboard');
    expect(bodyText).toContain('System Health');
    expect(bodyText).toContain('Active Runs');
  });

  test('capture task graph page', async ({ page }) => {
    await waitForApp(page);
    await page.evaluate(() => { window.location.hash = '#/project/project-1/graph'; });
    await page.waitForTimeout(500);
    await page.screenshot({ path: path.join(SCREENSHOT_DIR, 'task-graph.png'), fullPage: false });
    const bodyText = await page.textContent('body');
    expect(bodyText).toContain('Task Graph');
    expect(bodyText).toContain('COE-402');
  });

  test('capture run detail page', async ({ page }) => {
    await waitForApp(page);
    await page.evaluate(() => { window.location.hash = '#/run/run-001'; });
    await page.waitForTimeout(500);
    await page.screenshot({ path: path.join(SCREENSHOT_DIR, 'run-detail.png'), fullPage: false });
    const bodyText = await page.textContent('body');
    expect(bodyText).toContain('Run Detail');
    expect(bodyText).toContain('COE-402');
  });

  test('capture app shell with sidebar', async ({ page }) => {
    await waitForApp(page);
    // Sidebar starts open by default
    await expect(page.locator('aside')).toBeVisible({ timeout: 5000 });
    await page.screenshot({ path: path.join(SCREENSHOT_DIR, 'project-sidebar.png'), fullPage: false });
    const bodyText = await page.textContent('body');
    expect(bodyText).toContain('OpenSymphony-bootstrap');
    expect(bodyText).toContain('Dashboard');
    expect(bodyText).toContain('Projects');
  });

  test('capture command palette', async ({ page }) => {
    await waitForApp(page);
    // Open command palette with Ctrl+K (works in headless mode)
    await page.keyboard.press('Control+K');
    await page.waitForTimeout(500);
    // Command palette renders as a dialog
    await page.screenshot({ path: path.join(SCREENSHOT_DIR, 'command-palette.png'), fullPage: false });
    const bodyText = await page.textContent('body');
    expect(bodyText).toContain('Command Palette');
  });

  test('capture responsive layout at mobile viewport', async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 667 });
    await waitForApp(page);
    await page.screenshot({ path: path.join(SCREENSHOT_DIR, 'dashboard-mobile.png'), fullPage: false });
    const bodyText = await page.textContent('body');
    expect(bodyText).toContain('Dashboard');
  });
});
