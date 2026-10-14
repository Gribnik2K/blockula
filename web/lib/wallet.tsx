"use client";

import { Client, Wallet } from "@blockula/sdk";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { RPC } from "./config";
import { clearHint, loadHint, saveHint, type Hint } from "./session";
import type { AccountView, Status } from "./types";

interface Ctx {
  client: Client;
  /** Has localStorage been read yet? (false during SSR/first paint) */
  ready: boolean;
  hint: Hint | null;
  wallet: Wallet | null;
  account: AccountView | null;
  status: Status | null;
  refresh: () => Promise<void>;
  /** Store the unlocked wallet in memory and its non-secret hints on disk. */
  unlocked: (wallet: Wallet, hint: Hint) => void;
  updateHint: (patch: Partial<Hint>) => void;
  lock: () => void;
  forget: () => void;
}

const WalletCtx = createContext<Ctx | null>(null);

export function WalletProvider({ children }: { children: ReactNode }) {
  const client = useMemo(() => new Client(RPC), []);
  const [ready, setReady] = useState(false);
  const [hint, setHint] = useState<Hint | null>(null);
  const [wallet, setWallet] = useState<Wallet | null>(null);
  const [account, setAccount] = useState<AccountView | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const walletRef = useRef<Wallet | null>(null);
  walletRef.current = wallet;

  useEffect(() => {
    setHint(loadHint());
    setReady(true);
  }, []);

  const refresh = useCallback(async () => {
    const st = await client.status().catch(() => null);
    if (st) setStatus(st as Status);
    const w = walletRef.current;
    if (!w || !hintRef.current?.account) return;
    const a = (await client.account(w.account).catch(() => null)) as AccountView | null;
    if (a && walletRef.current === w) {
      setAccount(a);
      const name = a.names[0];
      if (name && hintRef.current && hintRef.current.name !== name) {
        const h = { ...hintRef.current, name };
        saveHint(h);
        setHint(h);
      }
    }
  }, [client]);

  const hintRef = useRef<Hint | null>(null);
  hintRef.current = hint;

  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
  }, [refresh, wallet]);

  const unlocked = useCallback((w: Wallet, h: Hint) => {
    saveHint(h);
    hintRef.current = h;
    walletRef.current = w;
    setHint(h);
    setWallet(w);
    setAccount(null);
    refresh();
  }, [refresh]);

  const updateHint = useCallback((patch: Partial<Hint>) => {
    const h = { ...(hintRef.current as Hint), ...patch };
    saveHint(h);
    hintRef.current = h;
    setHint(h);
    refresh();
  }, [refresh]);

  const lock = useCallback(() => {
    setWallet(null);
    setAccount(null);
  }, []);

  const forget = useCallback(() => {
    clearHint();
    setHint(null);
    setWallet(null);
    setAccount(null);
  }, []);

  const value = { client, ready, hint, wallet, account, status, refresh, unlocked, updateHint, lock, forget };
  // Render client-side only: the chosen network and hints live in this browser, so server HTML
  // would disagree with the client on first paint.
  return <WalletCtx.Provider value={value}>{ready ? children : null}</WalletCtx.Provider>;
}

export function useWallet() {
  const c = useContext(WalletCtx);
  if (!c) throw new Error("useWallet outside WalletProvider");
  return c;
}

/** Hook for the wallet-required pages: the gate guarantees these are set. */
export function useUnlocked() {
  const c = useWallet();
  return { ...c, wallet: c.wallet as Wallet, hint: c.hint as Hint };
}
