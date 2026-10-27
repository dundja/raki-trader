import path from 'node:path'

import { expect, test, type Page } from '@playwright/test'

/**
 * Smoke test against the REAL stack: the dev server on :3000 and a running traderd on :8787.
 * It reads live data, so assertions are about shape (cards, rows, labels), not values.
 * The only mutating calls are Pause and Resume through the UI; it never clicks Kill.
 * It only resumes a bot it paused itself, or one the backend stopped on its own (safety, startup).
 * A manual pause, a Kill, a loss cap or a config halt is the operator's decision and is left alone.
 */

/** Stop kinds the backend sets by itself; resuming from these does not override the operator. */
const AUTOMATIC_STOPS = new Set(['safety', 'startup'])

async function screenshot(page: Page, name: string) {
  await page.screenshot({
    path: path.join(test.info().project.testDir, 'screenshots', `${name}.png`),
    fullPage: true,
  })
}

/** Records an observation in the report and in the console output. */
function note(type: string, description: string) {
  test.info().annotations.push({ type, description })
  console.log(`[${type}] ${description}`)
}

test('live page shows three window cards with a ticking countdown', async ({ page }) => {
  await page.goto('/')

  const cards = page.locator('[data-testid^="window-card-"]')
  await expect(cards).toHaveCount(3, { timeout: 10_000 })

  const countdown = cards.first().getByTestId('countdown')
  const before = await countdown.textContent()
  await expect(countdown).not.toHaveText(before ?? '', { timeout: 2_000 })

  await screenshot(page, 'live')
})

test('health page shows feeds and gate progress', async ({ page }) => {
  await page.goto('/health')

  const clob = page.getByTestId('feed-clob_market')
  // Cells after the source header: enabled, connected, ...
  await expect(clob.locator('td').nth(1)).toHaveText('yes', { timeout: 10_000 })
  await expect(page.locator('.gate-label')).toHaveText(/\d+ \/ 500/, { timeout: 10_000 })

  await screenshot(page, 'health')
})

test('pause then resume through the UI', async ({ page }) => {
  await page.goto('/health')

  const mode = page.getByTestId('control-mode')
  await expect(mode).toBeVisible({ timeout: 10_000 })
  const dialog = page.getByRole('dialog')

  const initial = (await mode.textContent())?.trim()
  if (initial === 'RUNNING') {
    await page.getByRole('button', { name: 'Pause', exact: true }).click()
    await dialog.getByRole('textbox').fill('e2e smoke')
    await dialog.getByRole('button', { name: 'Confirm' }).click()
    await expect(mode).toHaveText('PAUSED', { timeout: 3_000 })
    note('pause', 'paused from RUNNING')
  } else {
    // The safety loop can already have stopped trading (e.g. clock offset > 50 ms).
    await expect(page.getByRole('button', { name: 'Pause', exact: true })).toBeDisabled()
    const stopped = page.locator('.control-panel p').filter({ hasText: /^Stopped: / })
    const stopKind = /^Stopped: (\S+)/.exec((await stopped.textContent()) ?? '')?.[1] ?? '—'
    if (initial !== 'PAUSED' || !AUTOMATIC_STOPS.has(stopKind)) {
      // A manual pause, a Kill (or a stop awaiting acknowledgement) is the operator's decision.
      note('resume', `skipped: mode ${initial}, stopped: ${stopKind}; not resuming an operator stop`)
      await screenshot(page, 'health-control')
      return
    }
    note('pause', `already ${initial} (stopped: ${stopKind}); Pause is disabled, nothing to click`)
  }

  await page.getByRole('button', { name: 'Resume', exact: true }).click()
  await expect(dialog).toBeVisible()
  if (await dialog.getByRole('checkbox').isVisible()) {
    // An acknowledgement is the user's decision; the smoke test never gives it on their behalf.
    note('resume', 'needs an acknowledgement; cancelled without resuming')
    await dialog.getByRole('button', { name: 'Cancel' }).click()
    await screenshot(page, 'health-control')
    return
  }
  await dialog.getByRole('button', { name: 'Confirm' }).click()

  const refused = dialog.getByRole('alert')
  await expect(mode.filter({ hasText: /^RUNNING$/ }).or(refused)).toBeVisible({ timeout: 3_000 })
  if (await refused.isVisible()) {
    // Valid while a safety check fails, e.g. the clock offset is above 50 ms.
    await expect(refused).toHaveText(/^safety checks failing/)
    note('resume', `refused: ${await refused.textContent()}`)
    await expect(mode).not.toHaveText('RUNNING')
    await screenshot(page, 'health-control')
    await dialog.getByRole('button', { name: 'Cancel' }).click()
  } else {
    note('resume', 'resumed: mode is RUNNING')
    await screenshot(page, 'health-control')
  }
})

test('windows list opens a window detail', async ({ page }) => {
  await page.goto('/windows')

  const rows = page.locator('[data-testid^="window-row-"]')
  await expect(rows.first()).toBeVisible({ timeout: 10_000 })
  await screenshot(page, 'windows')

  // The newest rows are upcoming windows with no series yet; prefer a finished one to exercise the chart.
  const complete = rows.filter({ hasText: 'complete' })
  const row = (await complete.count()) > 0 ? complete.first() : rows.first()
  const slug = (await row.getAttribute('data-testid'))!.replace('window-row-', '')
  await row.getByRole('link').click()
  await expect(page).toHaveURL(new RegExp(`/windows/${slug}$`))

  const chart = page.getByTestId('price-chart')
  const empty = page.getByText('No series recorded for this window')
  await expect(chart.or(empty)).toBeVisible({ timeout: 10_000 })
  note('detail', (await chart.isVisible()) ? `${slug}: chart` : `${slug}: empty message`)

  await screenshot(page, 'window-detail')
})
