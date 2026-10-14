const DEFAULT_RPC = process.env.NEXT_PUBLIC_BLOCKULA_RPC || "http://localhost:7070";
const RPC_KEY = "blockula.rpc";

/**
 * Which node to talk to. `?rpc=https://node.example` switches nodes and is remembered in this
 * browser (`?rpc=default` resets), so one deployment works with a local node or any public one.
 */
function resolveRpc(): string {
  if (typeof window === "undefined") return DEFAULT_RPC;
  try {
    const fromUrl = new URLSearchParams(window.location.search).get("rpc");
    if (fromUrl === "default") localStorage.removeItem(RPC_KEY);
    else if (fromUrl && /^https?:\/\/[^\s]+$/.test(fromUrl)) localStorage.setItem(RPC_KEY, fromUrl);
    return localStorage.getItem(RPC_KEY) || DEFAULT_RPC;
  } catch {
    return DEFAULT_RPC;
  }
}

export const RPC = resolveRpc().replace(/\/$/, "");

export const explorer = {
  home: () => `${RPC}/#/`,
  tx: (id: string) => `${RPC}/#/tx/${id}`,
  account: (nameOrAddress: string) => `${RPC}/#/account/${encodeURIComponent(nameOrAddress)}`,
  block: (height: number) => `${RPC}/#/block/${height}`,
};
