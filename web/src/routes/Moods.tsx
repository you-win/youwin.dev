/**
 * How each month was written.
 *
 * Mood is recorded on every post and read by exactly one thing — the familiar,
 * which spends it on a kaomoji. This is the other direction: the field handed
 * back to the person filling it in.
 *
 * **Why eight small charts rather than one stacked bar.** A stack was the
 * obvious form and it is not safe here. Stacked segments have to be told apart
 * by colour where they touch, and a month with no `excited` posts puts
 * non-adjacent slots against each other — so every pair has to separate, not
 * just the neighbours. Seven categorical hues cannot do that: measured under
 * simulated deuteranopia, `tired` (aqua) and `melancholy` (magenta) land 1.6
 * apart on a scale where 8 is the target and 6 the floor. They are the same
 * colour to a red-green colourblind reader. Faceting into one single-series row
 * per mood removes the problem rather than mitigating it — a row is one hue
 * throughout, so no two hues ever touch, and identity comes from the row's own
 * label. It also answers the question a timeline is actually asked ("has this
 * shifted?") better than a stack does.
 *
 * **Why shares rather than counts.** Most posts have no mood picked, so on an
 * absolute scale the "did not say" row would flatten the other seven into
 * nothing. Every cell is that mood's share of its month, every row runs 0–100%,
 * and the counts live in the table.
 */

import { createResource, For, Show, createSignal } from "solid-js";

import { api, MOOD_LABEL, MOODS, type Mood, type MoodMonth } from "../lib/api";

/**
 * The seven hues, in `Mood::ALL` order, plus the neutral for "did not say".
 *
 * Steps chosen for a dark surface and validated against this app's own card
 * colour (`base-200`, #101b15) rather than assumed: lightness band, chroma
 * floor, adjacent-pair separation under simulated protanopia and deuteranopia,
 * and 3:1 contrast against the surface all pass. Only the dark column exists
 * because `theme.css` sets `color-scheme: dark` — mistwood has no light mode to
 * step a second set for.
 *
 * The order is the safety mechanism, not decoration. It is fixed, and a mood
 * keeps its colour whatever else is on screen.
 */
const MOOD_COLOR: Record<Mood, string> = {
  content: "#3987e5",
  contemplative: "#d95926",
  tired: "#199e70",
  excited: "#c98500",
  melancholy: "#d55181",
  chaos: "#008300",
  neutral: "#9085e9",
};

/**
 * "Did not say" is not an eighth mood, and does not get a hue.
 *
 * It is the absence of a pick — the thing the familiar reads as permission to
 * infer — so it wears the muted ink every other non-data element wears. A
 * categorical colour here would make it look like a choice somebody made.
 */
const UNSAID_COLOR = "#898781";

/**
 * Months shown at once.
 *
 * Past this the columns are too thin to read on a phone. The cap is stated on
 * the page when it bites rather than silently dropping the tail — a chart that
 * quietly shows two years of a five-year archive is worse than one that says so.
 */
const MAX_MONTHS = 24;

/** One month's worth of one row. */
interface Cell {
  /** `August 2026` — carried on the cell so nothing has to index two arrays. */
  month: string;
  count: number;
  /** Every post that month, whatever its mood. */
  monthTotal: number;
  /** `count / monthTotal`, and 0 for a month with nothing in it. */
  share: number;
}

/** One row of the chart: a mood, its colour, and how each month went. */
interface Row {
  key: string;
  label: string;
  color: string;
  /** Oldest month first — a time axis reads left to right. */
  cells: Cell[];
  total: number;
}

/** One month of the table, with every row's count already attached. */
interface Column {
  month: string;
  total: number;
  counts: { key: string; count: number }[];
}

/**
 * Both shapes, built once.
 *
 * The chart wants rows-of-months and the table wants months-of-rows, and
 * deriving the second from the first in the markup means indexing one array by
 * another's position — which is exactly where an off-by-one hides in plain
 * sight. Two prepared structures cost a few lines and make both views loops
 * over their own data.
 */
function buildView(months: MoodMonth[]): { rows: Row[]; columns: Column[] } {
  const cellsFor = (count: (month: MoodMonth) => number): Cell[] =>
    months.map((month) => ({
      month: month.label,
      count: count(month),
      monthTotal: month.total,
      share: month.total > 0 ? count(month) / month.total : 0,
    }));

  const row = (key: string, label: string, color: string, cells: Cell[]): Row => ({
    key,
    label,
    color,
    cells,
    total: cells.reduce((sum, cell) => sum + cell.count, 0),
  });

  const rows: Row[] = [
    ...MOODS.map((mood) =>
      row(
        mood,
        MOOD_LABEL[mood],
        MOOD_COLOR[mood],
        cellsFor((month) => month.moods.find((e) => e.mood === mood)?.posts ?? 0),
      ),
    ),
    // Last, and separated: it is the absence of the seven above it.
    row("unsaid", "Did not say", UNSAID_COLOR, cellsFor((month) => month.unsaid)),
  ];

  // Newest first, matching every other list in the app. The chart runs the
  // other way because a time axis has to.
  const columns: Column[] = months
    .map((month, index) => ({
      month: month.label,
      total: month.total,
      counts: rows.map((r) => ({ key: r.key, count: r.cells[index]?.count ?? 0 })),
    }))
    .reverse();

  return { rows, columns };
}

export default function Moods() {
  const [data] = createResource(() => api.moods());
  const [showNumbers, setShowNumbers] = createSignal(false);

  /** Oldest first: time reads left to right. The API sends newest first. */
  const months = () => {
    const all = data()?.months ?? [];
    return all.slice(0, MAX_MONTHS).reverse();
  };

  const truncated = () => (data()?.months.length ?? 0) - months().length;
  const view = () => buildView(months());
  const posts = () => months().reduce((sum, m) => sum + m.total, 0);

  return (
    <div class="flex flex-col gap-6">
      <div class="flex flex-wrap items-baseline justify-between gap-2">
        <h1 class="text-lg font-medium">Moods</h1>
        <button
          type="button"
          class="btn btn-ghost btn-xs"
          onClick={() => setShowNumbers((was) => !was)}
        >
          {showNumbers() ? "Show the chart" : "Show the numbers"}
        </button>
      </div>

      <Show
        when={!data.loading}
        fallback={<p class="text-sm text-secondary">Loading…</p>}
      >
        <Show
          when={months().length > 0}
          fallback={
            <p class="text-sm text-secondary">
              Nothing written yet. Mood is the picker beside the visibility
              select in the composer; it feeds the familiar and never shows on
              youwin.dev.
            </p>
          }
        >
          <p class="text-sm text-secondary">
            Each row is one mood's share of the posts written that month, oldest
            on the left. {posts()} posts across {months().length}{" "}
            {months().length === 1 ? "month" : "months"}
            {truncated() > 0
              ? `, with ${truncated()} older ${truncated() === 1 ? "month" : "months"} not shown`
              : ""}
            .
          </p>

          <Show
            when={!showNumbers()}
            fallback={<Numbers rows={view().rows} columns={view().columns} />}
          >
            <Chart rows={view().rows} />
          </Show>
        </Show>
      </Show>
    </div>
  );
}

function Chart(props: { rows: Row[] }) {
  const span = () => {
    const cells = props.rows[0]?.cells ?? [];
    return { first: cells.at(0)?.month, last: cells.at(-1)?.month };
  };

  return (
    <div class="flex flex-col gap-3">
      <For each={props.rows}>
        {(row) => (
          <section
            class="flex flex-col gap-1"
            // The one place a row is set apart from the seven real moods.
            classList={{ "mt-2 border-t border-base-300 pt-3": row.key === "unsaid" }}
          >
            <div class="flex items-baseline justify-between gap-2 text-sm">
              <span class="flex items-center gap-2">
                {/* Identity rides a swatch beside the text, never the text
                    itself — a mid-lightness hue is illegible as body copy on
                    this surface. */}
                <span
                  class="inline-block size-2.5 shrink-0 rounded-full"
                  style={{ "background-color": row.color }}
                  aria-hidden="true"
                />
                <span>{row.label}</span>
              </span>
              <span class="tabular-nums text-secondary">{row.total}</span>
            </div>

            {/* One bar per month. Same hue throughout, so nothing here has to
                be told apart from anything beside it by colour. */}
            <div
              class="flex h-7 items-end gap-[2px]"
              role="img"
              aria-label={describe(row)}
            >
              <For each={row.cells}>
                {(cell) => (
                  <div
                    class="min-w-0 flex-1 rounded-t-[4px] bg-current"
                    style={{
                      color: row.color,
                      // A 2% floor so a month with none of this mood still
                      // shows a stub — a gap reads as missing data rather than
                      // as a real zero. The opacity is what says which it is.
                      height: `${Math.max(cell.share * 100, 2)}%`,
                      opacity: cell.count === 0 ? 0.25 : 1,
                    }}
                    title={`${cell.month}: ${cell.count} of ${cell.monthTotal} (${Math.round(
                      cell.share * 100,
                    )}%)`}
                  />
                )}
              </For>
            </div>
          </section>
        )}
      </For>

      {/* Only the ends are labelled. A tick under every column would be
          unreadable at this width, and the tooltip and the table carry the
          rest. */}
      <div class="flex justify-between text-xs text-secondary">
        <span>{span().first}</span>
        <Show when={span().last !== span().first}>
          <span>{span().last}</span>
        </Show>
      </div>
    </div>
  );
}

/** The row as a sentence, for a reader who cannot see the bars. */
function describe(row: Row): string {
  if (row.total === 0) return `${row.label}: none, in any month shown.`;

  const peak = row.cells.reduce((best, cell) =>
    cell.share > best.share ? cell : best,
  );

  return (
    `${row.label}: ${row.total} posts across ${row.cells.length} months, ` +
    `highest in ${peak.month} at ${Math.round(peak.share * 100)}%.`
  );
}

/**
 * The same data as a table.
 *
 * Not a fallback — the exact counts are genuinely easier to read here, and a
 * chart whose numbers are only available by hovering is a chart that excludes
 * anyone not using a mouse.
 */
function Numbers(props: { rows: Row[]; columns: Column[] }) {
  return (
    // The one thing on this page that can exceed the column width, so it
    // scrolls inside itself rather than widening the page.
    <div class="overflow-x-auto">
      <table class="w-full text-sm">
        <thead>
          <tr class="border-b border-base-300 text-left text-secondary">
            <th class="py-2 pr-4 font-medium">Month</th>
            <th class="py-2 pr-4 text-right font-medium">Posts</th>
            <For each={props.rows}>
              {(row) => (
                <th class="py-2 pr-4 text-right font-medium whitespace-nowrap">
                  <span
                    class="mr-1 inline-block size-2 rounded-full align-middle"
                    style={{ "background-color": row.color }}
                    aria-hidden="true"
                  />
                  {row.label}
                </th>
              )}
            </For>
          </tr>
        </thead>
        <tbody>
          <For each={props.columns}>
            {(column) => (
              <tr class="border-b border-base-300/50">
                <th class="py-2 pr-4 text-left font-normal whitespace-nowrap">
                  {column.month}
                </th>
                <td class="py-2 pr-4 text-right tabular-nums">{column.total}</td>
                <For each={column.counts}>
                  {(cell) => (
                    <td class="py-2 pr-4 text-right tabular-nums">
                      {/* A zero recedes rather than disappearing: the shape of
                          the table is part of what it is saying. */}
                      <span classList={{ "text-base-content/30": cell.count === 0 }}>
                        {cell.count}
                      </span>
                    </td>
                  )}
                </For>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  );
}
