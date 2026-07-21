/**
 * A tiny, dependency-free Markdown renderer for the in-app manual.
 *
 * It covers exactly the subset the docs in `/docs` use — headings, paragraphs,
 * bold/italic/inline-code, links, ordered and unordered lists, blockquotes,
 * fenced code blocks, horizontal rules, and pipe tables. The docs are the
 * single source of truth (also viewable on GitHub); this just presents them
 * inside the app so we never pull in a full Markdown stack. Content is our own,
 * bundled at build time, so there's no untrusted input to sanitize.
 */

import { Fragment, type ReactNode } from "react";

export function Markdown({ source }: { source: string }) {
  return <div className="markdown">{parseBlocks(source)}</div>;
}

/** Inline formatting: `code`, **bold**, *italic*, and [links](url). */
function renderInline(text: string): ReactNode[] {
  const patterns: {
    re: RegExp;
    make: (m: RegExpMatchArray) => ReactNode;
  }[] = [
    { re: /`([^`]+)`/, make: (m) => <code>{m[1]}</code> },
    { re: /\*\*(.+?)\*\*/, make: (m) => <strong>{renderInline(m[1])}</strong> },
    {
      re: /\[(.+?)\]\((.+?)\)/,
      make: (m) => {
        // Cross-references between doc files (e.g. "dongle-setup.md") aren't
        // navigable in-app; render them as plain emphasis instead of dead
        // links. External URLs open in the browser.
        const href = m[2];
        if (/^https?:\/\//.test(href)) {
          return (
            <a href={href} target="_blank" rel="noreferrer">
              {renderInline(m[1])}
            </a>
          );
        }
        return <em>{renderInline(m[1])}</em>;
      },
    },
    { re: /\*(.+?)\*/, make: (m) => <em>{renderInline(m[1])}</em> },
  ];

  const nodes: ReactNode[] = [];
  let rest = text;
  let key = 0;
  while (rest.length > 0) {
    let best: { idx: number; len: number; node: ReactNode } | null = null;
    for (const p of patterns) {
      const m = rest.match(p.re);
      if (m && m.index !== undefined && (best === null || m.index < best.idx)) {
        best = { idx: m.index, len: m[0].length, node: p.make(m) };
      }
    }
    if (!best) {
      nodes.push(<Fragment key={key++}>{rest}</Fragment>);
      break;
    }
    if (best.idx > 0) {
      nodes.push(<Fragment key={key++}>{rest.slice(0, best.idx)}</Fragment>);
    }
    nodes.push(<Fragment key={key++}>{best.node}</Fragment>);
    rest = rest.slice(best.idx + best.len);
  }
  return nodes;
}

function isSpecial(line: string): boolean {
  return (
    line.startsWith("#") ||
    line.startsWith("```") ||
    line.startsWith(">") ||
    /^\s*[-*]\s+/.test(line) ||
    /^\s*\d+\.\s+/.test(line) ||
    /^---+$/.test(line.trim()) ||
    line.includes("|")
  );
}

/**
 * Collect the items of a list starting at `start`, handling two Markdown
 * niceties: soft-wrapped item text (a plain continuation line joins the
 * previous item) and "loose" lists (a blank line between items doesn't end the
 * list, as long as another item of the same kind follows). Returns the items
 * and the index of the first line after the list.
 */
function collectListItems(
  lines: string[],
  start: number,
  marker: RegExp,
): { items: string[]; next: number } {
  const items: string[] = [];
  let i = start;
  while (i < lines.length) {
    const l = lines[i];
    if (marker.test(l)) {
      items.push(l.replace(marker, ""));
      i++;
    } else if (l.trim() === "") {
      // Blank line: keep going only if another item of this list follows.
      let j = i + 1;
      while (j < lines.length && lines[j].trim() === "") j++;
      if (j < lines.length && marker.test(lines[j])) {
        i = j;
      } else {
        break;
      }
    } else if (!isSpecial(l) && items.length > 0) {
      items[items.length - 1] += " " + l.trim();
      i++;
    } else {
      break;
    }
  }
  return { items, next: i };
}

function splitRow(line: string): string[] {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|")) s = s.slice(0, -1);
  return s.split("|").map((c) => c.trim());
}

function parseBlocks(md: string): ReactNode[] {
  const lines = md.replace(/\r\n/g, "\n").split("\n");
  const blocks: ReactNode[] = [];
  let i = 0;
  let key = 0;
  const push = (node: ReactNode) => blocks.push(<Fragment key={key++}>{node}</Fragment>);

  while (i < lines.length) {
    const line = lines[i];

    if (line.trim() === "") {
      i++;
      continue;
    }

    // Fenced code block.
    if (line.startsWith("```")) {
      const buf: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) {
        buf.push(lines[i]);
        i++;
      }
      i++; // closing fence
      push(
        <pre>
          <code>{buf.join("\n")}</code>
        </pre>,
      );
      continue;
    }

    // Heading (# .. ###).
    const h = line.match(/^(#{1,3})\s+(.*)$/);
    if (h) {
      const content = renderInline(h[2].trim());
      if (h[1].length === 1) push(<h1>{content}</h1>);
      else if (h[1].length === 2) push(<h2>{content}</h2>);
      else push(<h3>{content}</h3>);
      i++;
      continue;
    }

    // Horizontal rule.
    if (/^---+$/.test(line.trim())) {
      push(<hr />);
      i++;
      continue;
    }

    // Blockquote (one or more `>` lines) → callout.
    if (line.startsWith(">")) {
      const buf: string[] = [];
      while (i < lines.length && lines[i].startsWith(">")) {
        buf.push(lines[i].replace(/^>\s?/, ""));
        i++;
      }
      push(<blockquote>{renderInline(buf.join(" "))}</blockquote>);
      continue;
    }

    // Pipe table: a `|` line followed by a `---|---` separator.
    if (
      line.includes("|") &&
      i + 1 < lines.length &&
      lines[i + 1].includes("-") &&
      /^[\s:|-]+$/.test(lines[i + 1])
    ) {
      const header = splitRow(line);
      i += 2; // header + separator
      const rows: string[][] = [];
      while (i < lines.length && lines[i].trim() !== "" && lines[i].includes("|")) {
        rows.push(splitRow(lines[i]));
        i++;
      }
      push(
        <table>
          <thead>
            <tr>
              {header.map((c, ci) => (
                <th key={ci}>{renderInline(c)}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((r, ri) => (
              <tr key={ri}>
                {r.map((c, ci) => (
                  <td key={ci}>{renderInline(c)}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>,
      );
      continue;
    }

    // Unordered list.
    if (/^\s*[-*]\s+/.test(line)) {
      const items = collectListItems(lines, i, /^\s*[-*]\s+/);
      i = items.next;
      push(
        <ul>
          {items.items.map((it, ii) => (
            <li key={ii}>{renderInline(it)}</li>
          ))}
        </ul>,
      );
      continue;
    }

    // Ordered list.
    if (/^\s*\d+\.\s+/.test(line)) {
      const items = collectListItems(lines, i, /^\s*\d+\.\s+/);
      i = items.next;
      push(
        <ol>
          {items.items.map((it, ii) => (
            <li key={ii}>{renderInline(it)}</li>
          ))}
        </ol>,
      );
      continue;
    }

    // Paragraph: gather consecutive plain lines.
    const buf: string[] = [];
    while (i < lines.length && lines[i].trim() !== "" && !isSpecial(lines[i])) {
      buf.push(lines[i]);
      i++;
    }
    push(<p>{renderInline(buf.join(" "))}</p>);
  }

  return blocks;
}
