// Renders assets/icon.svg into the PNG sizes used by the packages.
// Usage: node scripts/render-icons.mjs   (needs `npm i -g playwright`)
import { chromium } from 'playwright';
import fs from 'fs';
import path from 'path';

const root = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
const svg = fs.readFileSync(path.join(root, 'assets/icon.svg'), 'utf8');
const out = path.join(root, 'assets/icons');
fs.mkdirSync(out, { recursive: true });

const browser = await chromium.launch();
const page = await browser.newPage();
for (const n of [16, 24, 32, 48, 64, 128, 256, 512, 1024]) {
  await page.setViewportSize({ width: n, height: n });
  const sized = svg.replace('width="512" height="512"', `width="${n}" height="${n}"`);
  await page.setContent(`<html><body style="margin:0;background:transparent">${sized}</body></html>`);
  await page.screenshot({ path: path.join(out, `icon-${n}.png`), omitBackground: true, clip: { x: 0, y: 0, width: n, height: n } });
}
await browser.close();
