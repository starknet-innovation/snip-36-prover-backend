import { useState, useCallback } from "react";
import { hash } from "starknet";
import { api } from "./lib/api";
import CoinDisplay from "./components/CoinDisplay";
import WalletButton from "./components/WalletButton";
import LogPanel from "./components/LogPanel";

interface FlipResult {
  outcome: string;
  bet: string;
  won: boolean;
  tx_hash: string;
  proof_size: number;
}

interface WalletOption {
  id: string;
  name: string;
  icon?: string | { dark: string; light: string };
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  _raw: any;
}

interface GameState {
  address: string | null;
  bet: 0 | 1;
  phase: string | null;
  logs: string[];
  playing: boolean;
  deploying: boolean;
  result: FlipResult | null;
  error: string | null;
  history: FlipResult[];
  walletPicker: WalletOption[] | null;
}

const sectionStyle: React.CSSProperties = {
  background: "rgba(255,255,255,0.03)",
  border: "1px solid rgba(255,255,255,0.08)",
  borderRadius: 10,
  padding: "12px 16px",
  marginBottom: 8,
  textAlign: "left",
};

const stepNumStyle: React.CSSProperties = {
  display: "inline-block",
  width: 22,
  height: 22,
  lineHeight: "22px",
  borderRadius: "50%",
  background: "rgba(255, 215, 0, 0.2)",
  color: "#ffd700",
  fontSize: 12,
  fontWeight: 700,
  textAlign: "center",
  marginRight: 8,
  flexShrink: 0,
};

const labelStyle: React.CSSProperties = {
  fontSize: 13,
  fontWeight: 600,
  color: "#e0e0e0",
  marginBottom: 4,
};

const descStyle: React.CSSProperties = {
  fontSize: 12,
  color: "#888",
  lineHeight: 1.5,
};

function HowItWorks() {
  const [open, setOpen] = useState(false);

  return (
    <div style={{ marginTop: 32, textAlign: "left" }}>
      <button
        onClick={() => setOpen((o) => !o)}
        style={{
          background: "none",
          border: "none",
          color: "#ffd700",
          cursor: "pointer",
          fontSize: 13,
          fontWeight: 600,
          padding: 0,
          display: "flex",
          alignItems: "center",
          gap: 6,
        }}
      >
        {open ? "Hide" : "How it works"} {open ? "\u25B2" : "\u25BC"}
      </button>

      {open && (
        <div style={{ marginTop: 12 }}>
          <div style={sectionStyle}>
            <div style={{ display: "flex", alignItems: "flex-start" }}>
              <span style={stepNumStyle}>1</span>
              <div>
                <div style={labelStyle}>Commit (player)</div>
                <div style={descStyle}>
                  You pick heads or tails. Your browser generates a random nonce
                  and computes <code style={{ color: "#aaa" }}>pedersen(bet, nonce)</code> — a
                  cryptographic commitment that hides your bet. Only this hash is
                  sent to the server. The server cannot see your bet.
                </div>
              </div>
            </div>
          </div>

          <div style={sectionStyle}>
            <div style={{ display: "flex", alignItems: "flex-start" }}>
              <span style={stepNumStyle}>2</span>
              <div>
                <div style={labelStyle}>Lock seed (server)</div>
                <div style={descStyle}>
                  The server records the current Starknet block number as the
                  seed. This happens <em>after</em> your commitment is locked, so the
                  server cannot pick a block that favors a particular outcome —
                  it doesn't know your bet yet.
                </div>
              </div>
            </div>
          </div>

          <div style={sectionStyle}>
            <div style={{ display: "flex", alignItems: "flex-start" }}>
              <span style={stepNumStyle}>3</span>
              <div>
                <div style={labelStyle}>Reveal (player)</div>
                <div style={descStyle}>
                  Your browser sends the actual bet and nonce. The server
                  verifies <code style={{ color: "#aaa" }}>pedersen(bet, nonce) == commitment</code>.
                  If it doesn't match, the game is rejected — you can't change
                  your bet after seeing the seed.
                </div>
              </div>
            </div>
          </div>

          <div style={sectionStyle}>
            <div style={{ display: "flex", alignItems: "flex-start" }}>
              <span style={stepNumStyle}>4</span>
              <div>
                <div style={labelStyle}>Prove in Virtual OS (server)</div>
                <div style={descStyle}>
                  The server constructs a transaction calling{" "}
                  <code style={{ color: "#aaa" }}>play(seed, player, bet)</code> on
                  the CoinFlip contract. It executes this off-chain in a SNIP-36
                  virtual OS and generates a STARK proof (via stwo prover). The
                  outcome is <code style={{ color: "#aaa" }}>pedersen(seed, player_address) & 1</code> —
                  fully deterministic from public inputs.
                </div>
              </div>
            </div>
          </div>

          <div style={sectionStyle}>
            <div style={{ display: "flex", alignItems: "flex-start" }}>
              <span style={stepNumStyle}>5</span>
              <div>
                <div style={labelStyle}>Settlement message</div>
                <div style={descStyle}>
                  The CoinFlip contract emits an L2-to-L1 message with the
                  settlement receipt: <code style={{ color: "#aaa" }}>[player, seed, bet, outcome, won]</code>.
                  This message is part of the proven execution trace — it cannot
                  be forged or tampered with.
                </div>
              </div>
            </div>
          </div>

          <div style={sectionStyle}>
            <div style={{ display: "flex", alignItems: "flex-start" }}>
              <span style={stepNumStyle}>6</span>
              <div>
                <div style={labelStyle}>Submit proof on-chain</div>
                <div style={descStyle}>
                  The STARK proof and proof_facts are submitted as a
                  proof-bearing transaction to Starknet. The on-chain verifier
                  confirms the proof is valid — guaranteeing the game was played
                  honestly.
                </div>
              </div>
            </div>
          </div>

          <div
            style={{
              ...sectionStyle,
              background: "rgba(255, 215, 0, 0.05)",
              border: "1px solid rgba(255, 215, 0, 0.15)",
            }}
          >
            <div style={{ ...labelStyle, color: "#ffd700" }}>
              Why can't anyone cheat?
            </div>
            <div style={descStyle}>
              <strong style={{ color: "#ccc" }}>Player</strong>: Your bet is
              committed before the seed is revealed. You cannot change it after
              seeing the outcome.
              <br />
              <strong style={{ color: "#ccc" }}>Server</strong>: The seed is
              locked after your commitment. It cannot pick a favorable block.
              The STARK proof guarantees the contract was executed correctly — a
              fake outcome would produce an invalid proof.
              <br />
              <strong style={{ color: "#ccc" }}>Anyone</strong>: Given the seed
              (block number) and player address, anyone can compute{" "}
              <code style={{ color: "#aaa" }}>pedersen(seed, player) & 1</code>{" "}
              and independently verify the result.
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default function App() {
  const [state, setState] = useState<GameState>({
    address: null,
    bet: 0,
    phase: null,
    logs: [],
    playing: false,
    deploying: false,
    result: null,
    error: null,
    history: [],
    walletPicker: null,
  });

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const snRef = { current: null as any };

  const handleConnect = useCallback(async () => {
    try {
      const { getStarknet } = await import("get-starknet-core");
      const sn = getStarknet();
      snRef.current = sn;
      const wallets = await sn.getAvailableWallets();
      if (wallets.length === 0) {
        setState((s) => ({
          ...s,
          error: "No Starknet wallet found. Install ArgentX or Braavos.",
        }));
        return;
      }
      if (wallets.length === 1) {
        await connectWallet(sn, wallets[0]);
      } else {
        // Show picker
        setState((s) => ({
          ...s,
          walletPicker: wallets.map((w) => ({
            id: w.id,
            name: w.name || w.id,
            icon: w.icon,
            _raw: w,
          })),
        }));
      }
    } catch (e) {
      console.error("Wallet connection failed:", e);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const connectWallet = async (sn: any, wallet: any) => {
    const enabled = await sn.enable(wallet);
    const accounts: string[] = await enabled.request({
      type: "wallet_requestAccounts",
    });
    if (accounts.length > 0) {
      setState((s) => ({
        ...s,
        address: accounts[0],
        error: null,
        walletPicker: null,
      }));
    }
  };

  const handlePickWallet = useCallback(
    async (wallet: WalletOption) => {
      try {
        const { getStarknet } = await import("get-starknet-core");
        const sn = snRef.current || getStarknet();
        await connectWallet(sn, wallet._raw);
      } catch (e) {
        console.error("Wallet connection failed:", e);
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  const handleDisconnect = useCallback(() => {
    setState((s) => ({ ...s, address: null, result: null }));
  }, []);

  const handleFlip = useCallback(async () => {
    if (!state.address) return;

    setState((s) => ({
      ...s,
      playing: true,
      result: null,
      error: null,
      logs: [],
      phase: null,
    }));

    // Auto-deploy if needed
    try {
      const status = await api.coinflipStatus();
      if (!status.deployed) {
        setState((s) => ({ ...s, deploying: true, logs: [...s.logs, "Deploying CoinFlip contract (one-time setup)..."] }));
        await api.deployCoinflip();
        setState((s) => ({ ...s, deploying: false, logs: [...s.logs, "CoinFlip contract deployed."] }));
      }
    } catch (e) {
      setState((s) => ({
        ...s,
        playing: false,
        deploying: false,
        error: `Deploy failed: ${e}`,
      }));
      return;
    }

    // ── Commit-reveal: lock bet before seed is chosen ────
    // Generate random nonce
    const nonceBytes = new Uint8Array(31);
    crypto.getRandomValues(nonceBytes);
    const nonce =
      "0x" +
      Array.from(nonceBytes)
        .map((b) => b.toString(16).padStart(2, "0"))
        .join("");

    // Compute commitment = pedersen(bet, nonce)
    const commitment = hash.computePedersenHash(
      "0x" + state.bet.toString(16),
      nonce,
    );

    setState((s) => ({
      ...s,
      logs: [
        ...s.logs,
        `Committing bet (commitment: ${commitment.slice(0, 18)}...)`,
      ],
    }));

    let sessionId: string;
    try {
      const commitResp = await api.commit(commitment, state.address);
      sessionId = commitResp.session_id;
      setState((s) => ({
        ...s,
        logs: [
          ...s.logs,
          `Seed locked at block ${commitResp.seed_block} (commit-reveal: server cannot change seed)`,
        ],
      }));
    } catch (e) {
      setState((s) => ({
        ...s,
        playing: false,
        error: `Commit failed: ${e}`,
      }));
      return;
    }

    // Reveal bet + nonce and start play SSE
    const source = api.play(sessionId, state.address, state.bet, nonce);

    source.addEventListener("log", (e: MessageEvent) => {
      setState((s) => ({ ...s, logs: [...s.logs, e.data] }));
    });

    source.addEventListener("phase", (e: MessageEvent) => {
      setState((s) => ({ ...s, phase: e.data }));
    });

    source.addEventListener("result", (e: MessageEvent) => {
      const data: FlipResult = JSON.parse(e.data);
      setState((s) => ({
        ...s,
        playing: false,
        phase: null,
        result: data,
        history: [data, ...s.history],
      }));
      source.close();
    });

    source.addEventListener("error", (e: Event) => {
      const me = e as MessageEvent;
      const msg = me.data || "Connection lost";
      setState((s) => ({ ...s, playing: false, phase: null, error: msg }));
      source.close();
    });
  }, [state.address, state.bet]);

  const phaseLabel = (phase: string | null): string => {
    switch (phase) {
      case "constructing":
        return "Constructing transaction...";
      case "proving":
        return "Proving in virtual OS...";
      case "submitting":
        return "Submitting proof...";
      case "verifying":
        return "Waiting for confirmation...";
      default:
        return state.deploying ? "Setting up..." : "";
    }
  };

  const resultSide =
    state.result?.outcome === "heads"
      ? "heads"
      : state.result?.outcome === "tails"
      ? "tails"
      : null;

  return (
    <div
      style={{
        maxWidth: 480,
        margin: "0 auto",
        padding: "40px 20px",
        textAlign: "center",
      }}
    >
      {/* Header */}
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          marginBottom: 40,
        }}
      >
        <h1 style={{ fontSize: 22, fontWeight: 700, color: "#ffd700" }}>
          SNIP-36 CoinFlip
        </h1>
        <WalletButton
          address={state.address}
          onConnect={handleConnect}
          onDisconnect={handleDisconnect}
        />
      </div>

      {/* Wallet picker modal */}
      {state.walletPicker && (
        <div
          style={{
            position: "fixed",
            inset: 0,
            background: "rgba(0,0,0,0.7)",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            zIndex: 100,
          }}
          onClick={() => setState((s) => ({ ...s, walletPicker: null }))}
        >
          <div
            style={{
              background: "#1a1a2e",
              borderRadius: 16,
              padding: 24,
              minWidth: 280,
              border: "1px solid rgba(255,255,255,0.1)",
            }}
            onClick={(e) => e.stopPropagation()}
          >
            <h3 style={{ marginBottom: 16, fontSize: 16 }}>Choose a wallet</h3>
            {state.walletPicker.map((w) => (
              <button
                key={w.id}
                onClick={() => handlePickWallet(w)}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 12,
                  width: "100%",
                  padding: "12px 16px",
                  marginBottom: 8,
                  borderRadius: 10,
                  border: "1px solid rgba(255,255,255,0.1)",
                  background: "rgba(255,255,255,0.05)",
                  color: "#e0e0e0",
                  cursor: "pointer",
                  fontSize: 15,
                }}
              >
                {w.icon && (
                  <img
                    src={typeof w.icon === "string" ? w.icon : w.icon.dark}
                    alt=""
                    style={{ width: 28, height: 28, borderRadius: 6 }}
                  />
                )}
                {w.name}
              </button>
            ))}
          </div>
        </div>
      )}

      {!state.address ? (
        <div style={{ marginTop: 80 }}>
          <CoinDisplay side={null} spinning={false} />
          <p style={{ marginTop: 24, color: "#888", fontSize: 14 }}>
            Connect your Starknet wallet to play
          </p>
          <p style={{ marginTop: 8, color: "#666", fontSize: 12 }}>
            Provably fair coin flip powered by SNIP-36 virtual blocks
          </p>

          {/* How it works — always visible before connect */}
          <HowItWorks />
        </div>
      ) : (
        <>
          {/* Coin */}
          <CoinDisplay
            side={state.playing ? null : resultSide}
            spinning={state.playing}
          />

          {/* Result */}
          {state.result && !state.playing && (
            <div
              style={{
                marginTop: 24,
                padding: 16,
                borderRadius: 12,
                background: state.result.won
                  ? "rgba(0, 200, 80, 0.15)"
                  : "rgba(255, 60, 60, 0.15)",
                border: `1px solid ${
                  state.result.won
                    ? "rgba(0, 200, 80, 0.3)"
                    : "rgba(255, 60, 60, 0.3)"
                }`,
              }}
            >
              <div
                style={{
                  fontSize: 28,
                  fontWeight: 700,
                  color: state.result.won ? "#00c850" : "#ff3c3c",
                }}
              >
                {state.result.won ? "YOU WIN!" : "YOU LOSE"}
              </div>
              <div style={{ fontSize: 14, color: "#aaa", marginTop: 4 }}>
                Coin landed on{" "}
                <strong style={{ color: "#e0e0e0" }}>
                  {state.result.outcome}
                </strong>
                {" "}| You bet{" "}
                <strong style={{ color: "#e0e0e0" }}>{state.result.bet}</strong>
              </div>
              <div
                style={{
                  fontSize: 11,
                  color: "#666",
                  marginTop: 8,
                  fontFamily: "monospace",
                }}
              >
                proof: {(state.result.proof_size / 1024).toFixed(0)} KB | tx:{" "}
                {state.result.tx_hash.slice(0, 14)}...
              </div>
            </div>
          )}

          {/* Phase indicator */}
          {(state.playing || state.deploying) && (
            <div style={{ marginTop: 24, color: "#ffd700", fontSize: 14 }}>
              {phaseLabel(state.phase)}
            </div>
          )}

          {/* Error */}
          {state.error && (
            <div
              style={{
                marginTop: 16,
                padding: 12,
                borderRadius: 8,
                background: "rgba(255, 60, 60, 0.1)",
                border: "1px solid rgba(255, 60, 60, 0.3)",
                color: "#ff6b6b",
                fontSize: 13,
              }}
            >
              {state.error}
            </div>
          )}

          {/* Bet selector */}
          {!state.playing && (
            <div style={{ marginTop: 32 }}>
              <div
                style={{
                  display: "flex",
                  gap: 12,
                  justifyContent: "center",
                  marginBottom: 16,
                }}
              >
                {([0, 1] as const).map((b) => (
                  <button
                    key={b}
                    onClick={() =>
                      setState((s) => ({ ...s, bet: b, result: null }))
                    }
                    style={{
                      padding: "10px 28px",
                      borderRadius: 8,
                      border:
                        state.bet === b
                          ? "2px solid #ffd700"
                          : "2px solid rgba(255,255,255,0.15)",
                      background:
                        state.bet === b
                          ? "rgba(255, 215, 0, 0.15)"
                          : "rgba(255,255,255,0.05)",
                      color: state.bet === b ? "#ffd700" : "#aaa",
                      fontSize: 16,
                      fontWeight: 600,
                      cursor: "pointer",
                    }}
                  >
                    {b === 0 ? "Heads" : "Tails"}
                  </button>
                ))}
              </div>

              <button
                onClick={handleFlip}
                disabled={state.playing}
                style={{
                  padding: "14px 48px",
                  borderRadius: 12,
                  border: "none",
                  background:
                    "linear-gradient(135deg, #ffd700, #f0a000)",
                  color: "#1a1a2e",
                  fontSize: 18,
                  fontWeight: 700,
                  cursor: "pointer",
                  opacity: state.playing ? 0.5 : 1,
                }}
              >
                Flip Coin
              </button>
            </div>
          )}

          {/* Logs */}
          <LogPanel logs={state.logs} />

          {/* How it works */}
          <HowItWorks />

          {/* History */}
          {state.history.length > 0 && (
            <div style={{ marginTop: 32, textAlign: "left" }}>
              <h3
                style={{ fontSize: 14, color: "#888", marginBottom: 8 }}
              >
                History
              </h3>
              {state.history.map((h, i) => (
                <div
                  key={i}
                  style={{
                    display: "flex",
                    justifyContent: "space-between",
                    padding: "6px 10px",
                    borderRadius: 6,
                    background: "rgba(255,255,255,0.03)",
                    marginBottom: 4,
                    fontSize: 13,
                    fontFamily: "monospace",
                  }}
                >
                  <span>
                    bet:{h.bet} outcome:{h.outcome}
                  </span>
                  <span
                    style={{
                      color: h.won ? "#00c850" : "#ff3c3c",
                      fontWeight: 600,
                    }}
                  >
                    {h.won ? "WIN" : "LOSE"}
                  </span>
                </div>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}
