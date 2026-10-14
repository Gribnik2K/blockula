"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { explorer, RPC } from "@/lib/config";
import { useWallet } from "@/lib/wallet";

export function Header() {
  const { wallet, hint, lock } = useWallet();
  const path = usePathname();
  const router = useRouter();
  const open = wallet && hint?.account;
  return (
    <>
      <header className="top">
        <Link href="/" className="brand">
          🧛 Blockula <span className="dim">wallet</span>
        </Link>
        <nav>
          {open && (
            <>
              <Link href="/" className={path === "/" ? "active" : ""}>Home</Link>
              <Link href="/devices" className={path === "/devices" ? "active" : ""}>Devices &amp; agents</Link>
            </>
          )}
          <span className="dim small" title="The Blockula node this wallet talks to. Switch with ?rpc=https://your-node in the URL (?rpc=default resets).">
            🌐 {RPC.replace(/^https?:\/\//, "")}
          </span>
          <a href={explorer.home()} target="_blank" rel="noreferrer">Explorer ↗</a>
          {open && (
            <button
              className="link"
              title="Lock: forget the key in memory until you unlock again"
              onClick={() => {
                lock();
                router.push("/");
              }}
            >
              🔒
            </button>
          )}
        </nav>
      </header>
      {hint?.kind === "local" && (
        <div className="banner">
          ⚠️ <b>Browser-only key (testing).</b> Its secret is stored in this browser, not protected by a passkey. Don&apos;t keep real value here.
        </div>
      )}
    </>
  );
}
