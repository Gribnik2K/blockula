import type { Metadata, Viewport } from "next";
import type { ReactNode } from "react";
import { Header } from "@/components/Header";
import { WalletProvider } from "@/lib/wallet";
import "./globals.css";

export const metadata: Metadata = {
  title: "🧛 Blockula Wallet",
  description: "A post-quantum wallet unlocked by your passkey. No seed phrase, just a .boo name.",
};

export const viewport: Viewport = { themeColor: "#0e0a14", width: "device-width", initialScale: 1 };

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body>
        <WalletProvider>
          <Header />
          {children}
          <footer className="foot dim small">
            Quantum-safe ML-DSA-65 keys · unlocked by passkeys · not audited, test chains only 🕸️
          </footer>
        </WalletProvider>
      </body>
    </html>
  );
}
