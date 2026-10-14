"use client";

import { explorer } from "@/lib/config";
import type { TxView } from "@/lib/types";
import { ExplorerLink } from "./ui";

const icon = (t: TxView, me: string) => {
  if (t.kind === "register_name") return "🏷️";
  if (t.kind === "add_key") return "🔑";
  if (t.kind === "remove_key") return "🚫";
  return t.from.address === me ? "📤" : "📥";
};

export function Activity({ txs, me }: { txs: TxView[]; me: string }) {
  if (!txs.length) return <p className="dim">Nothing yet. Your transactions will appear here once they're sealed into a block. 🕸️</p>;
  return (
    <ul className="activity">
      {txs.map((t) => (
        <li key={t.id}>
          <ExplorerLink href={explorer.tx(t.id)} className="activity-row">
            <span className="activity-icon">{icon(t, me)}</span>
            <span className="activity-main">
              <span>{t.summary}</span>
              {t.memo && <span className="dim block">💬 “{t.memo}”</span>}
            </span>
            <span className="dim small nowrap">{t.height ? `#${t.height}` : "pending"} ↗</span>
          </ExplorerLink>
        </li>
      ))}
    </ul>
  );
}
