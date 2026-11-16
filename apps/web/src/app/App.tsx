import { ArrowUp, Scan, Star, WifiOff, ShieldCheck, Layers } from "lucide-react";

import { SendPage } from "../features/send/SendPage";
import { ReceivePage } from "../features/receive/ReceivePage";
import pkg from "../../package.json";

const REPO_URL = "https://github.com/Rudrakshbhardwaj01/Pigeon";
const PORTFOLIO_URL = "https://rudrakshbhardwaj.vercel.app/";

function scrollTo(id: string) {
  document.getElementById(id)?.scrollIntoView({ behavior: "smooth", block: "start" });
}

/** GitHub mark (Simple Icons path), sized like the surrounding lucide icons. */
function GithubIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" focusable="false">
      <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
    </svg>
  );
}

export function App() {
  return (
    <div className="app">
      <a href="#send" className="skip-link">
        Skip to content
      </a>

      <header className="hero" id="top">
        <div className="hero-inner">
          <h1 className="hero-wordmark">Pigeon</h1>
          <p className="hero-kicker">Optical file transfer</p>
          <p className="hero-tag">
            Send any file through animated QR codes. No server. No pairing.
            Just a GIF and a camera.
          </p>
          <div className="hero-actions">
            <button className="btn primary hero-btn" onClick={() => scrollTo("send")}>
              <ArrowUp size={17} /> Send a File
            </button>
            <button className="btn hero-btn hero-btn-outline" onClick={() => scrollTo("receive")}>
              <Scan size={17} /> Receive a File
            </button>
          </div>
          <a
            className="github-cta"
            href={REPO_URL}
            target="_blank"
            rel="noreferrer"
            aria-label="Star Pigeon on GitHub"
          >
            <GithubIcon size={16} />
            <span>Star Pigeon on GitHub</span>
            <Star size={15} aria-hidden="true" />
          </a>
          <ul className="hero-indicators">
            <li>
              <WifiOff size={14} /> Offline by design
            </li>
            <li>
              <ShieldCheck size={14} /> Private &amp; local
            </li>
            <li>
              <Layers size={14} /> Resilient transfer
            </li>
          </ul>
        </div>
      </header>

      <main className="app-main" id="main">
        <SendPage />
        <ReceivePage />
      </main>

      <footer className="app-footer">
        <span className="footer-wordmark">Pigeon</span>
        <nav className="footer-links" aria-label="Footer">
          <span>Files never leave this device</span>
          <a href={REPO_URL} target="_blank" rel="noreferrer">
            GitHub
          </a>
        </nav>
        <span className="footer-meta">
          Made by{" "}
          <a href={PORTFOLIO_URL} target="_blank" rel="noreferrer">
            Rudraksh Bhardwaj
          </a>
          <span className="mono footer-version"> · v{pkg.version}</span>
        </span>
      </footer>
    </div>
  );
}
