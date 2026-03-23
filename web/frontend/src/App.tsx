import { useState, useCallback } from "react";
import {
  generateKeyPair,
  truncateHex,
  type StarkKeyPair,
} from "./lib/starknet";
import { api } from "./lib/api";
import { StepCard } from "./components/StepCard";
import { CounterDisplay } from "./components/CounterDisplay";
import { LogPanel } from "./components/LogPanel";
import { Explainer } from "./components/Explainer";

type StepStatus = "pending" | "active" | "done" | "error";

interface BlockResult {
  blockIndex: number;
  txHash: string;
  counterBefore: number;
  counterAfter: number;
  increment: number;
  proofSize: number;
  durationMs: number;
}

interface AppState {
  sessionId: string;
  keyPair: StarkKeyPair | null;
  fundTxHash: string | null;
  accountDeployed: boolean;
  contractAddress: string | null;
  classHash: string | null;
  counterValue: number | null;
  proveLogs: string[];
  provePhase: string | null;
  blockHistory: BlockResult[];
  error: string | null;
}

const initialState: AppState = {
  sessionId: crypto.randomUUID(),
  keyPair: null,
  fundTxHash: null,
  accountDeployed: false,
  contractAddress: null,
  classHash: null,
  counterValue: null,
  proveLogs: [],
  provePhase: null,
  blockHistory: [],
  error: null,
};

export default function App() {
  const [state, setState] = useState<AppState>(initialState);
  const [steps, setSteps] = useState<Record<number, StepStatus>>({
    1: "active",
    2: "pending",
    3: "pending",
    4: "pending",
    5: "pending",
  });
  const [loading, setLoading] = useState(false);
  const [proving, setProving] = useState(false);
  const [incrementAmount, setIncrementAmount] = useState(1);
  const [incrementsPerBlock, setIncrementsPerBlock] = useState(1);

  const setStep = (n: number, status: StepStatus) =>
    setSteps((prev) => ({ ...prev, [n]: status }));

  const setError = (msg: string) => setState((s) => ({ ...s, error: msg }));

  // ── Step 1: Generate Key ──────────────────────────────

  const handleGenerateKey = useCallback(() => {
    const keyPair = generateKeyPair();
    setState((s) => ({ ...s, keyPair, error: null }));
    setStep(1, "done");
    setStep(2, "active");
  }, []);

  // ── Step 2: Fund Account ──────────────────────────────

  const handleFund = useCallback(async () => {
    if (!state.keyPair) return;
    setLoading(true);
    setError("");
    try {
      const result = await api.fund(state.keyPair.accountAddress);
      setState((s) => ({ ...s, fundTxHash: result.tx_hash }));
      setStep(2, "done");
      setStep(3, "active");
    } catch (e: any) {
      setError(e.message);
      setStep(2, "error");
    }
    setLoading(false);
  }, [state.keyPair]);

  // ── Step 3: Deploy Account ────────────────────────────

  const handleDeployAccount = useCallback(async () => {
    if (!state.keyPair) return;
    setLoading(true);
    setError("");
    try {
      await api.deployAccount(
        state.sessionId,
        state.keyPair.publicKey,
        state.keyPair.accountAddress
      );
      setState((s) => ({ ...s, accountDeployed: true }));
      setStep(3, "done");
      setStep(4, "active");
    } catch (e: any) {
      setError(e.message);
      setStep(3, "error");
    }
    setLoading(false);
  }, [state.keyPair, state.sessionId]);

  // ── Step 4: Deploy Counter ────────────────────────────

  const handleDeployCounter = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const result = await api.deployCounter(state.sessionId);
      setState((s) => ({
        ...s,
        contractAddress: result.contract_address,
        classHash: result.class_hash,
        counterValue: 0,
      }));
      setStep(4, "done");
      setStep(5, "active");
    } catch (e: any) {
      setError(e.message);
      setStep(4, "error");
    }
    setLoading(false);
  }, [state.sessionId]);

  // ── Step 5: Prove & Submit SNOS Block ─────────────────

  const handleProveBlock = useCallback(async () => {
    if (proving) return;
    setProving(true);
    setState((s) => ({
      ...s,
      proveLogs: [],
      provePhase: "constructing",
      error: null,
    }));

    const counterBefore = state.counterValue ?? 0;
    const blockStart = Date.now();
    const blockIndex = state.blockHistory.length + 1;

    const source = api.proveBlock(
      state.sessionId,
      incrementAmount,
      incrementsPerBlock
    );

    source.addEventListener("log", (e: MessageEvent) => {
      setState((s) => ({ ...s, proveLogs: [...s.proveLogs, e.data] }));
    });

    source.addEventListener("phase", (e: MessageEvent) => {
      setState((s) => ({ ...s, provePhase: e.data }));
    });

    source.addEventListener("complete", (e: MessageEvent) => {
      source.close();
      const data = JSON.parse(e.data);
      const durationMs = Date.now() - blockStart;

      const result: BlockResult = {
        blockIndex,
        txHash: data.tx_hash,
        counterBefore,
        counterAfter: data.counter_value,
        increment: data.increment,
        proofSize: data.proof_size,
        durationMs,
      };

      setState((s) => ({
        ...s,
        counterValue: data.counter_value,
        provePhase: null,
        blockHistory: [...s.blockHistory, result],
      }));
      setProving(false);
    });

    source.addEventListener("error", (e: MessageEvent) => {
      source.close();
      setError(e.data || "Prove-and-submit failed");
      setState((s) => ({ ...s, provePhase: null }));
      setProving(false);
    });

    source.onerror = () => {
      source.close();
      setProving(false);
    };
  }, [
    state.sessionId,
    state.counterValue,
    state.blockHistory.length,
    incrementAmount,
    incrementsPerBlock,
    proving,
  ]);

  // ── Refresh counter ───────────────────────────────────

  const refreshCounter = useCallback(async () => {
    if (!state.contractAddress) return;
    try {
      const { counter_value } = await api.readCounter(state.contractAddress);
      setState((s) => ({ ...s, counterValue: counter_value }));
    } catch {}
  }, [state.contractAddress]);

  // ── Phase display ─────────────────────────────────────

  const phaseLabel = (phase: string | null): string => {
    switch (phase) {
      case "constructing":
        return "Constructing transaction...";
      case "proving":
        return "Proving in virtual OS...";
      case "submitting":
        return "Submitting via RPC...";
      case "verifying":
        return "Waiting for on-chain confirmation...";
      default:
        return "";
    }
  };

  const expectedIncrement = incrementAmount * incrementsPerBlock;

  // ── Render ────────────────────────────────────────────

  return (
    <div
      style={{
        maxWidth: 720,
        margin: "0 auto",
        padding: "32px 16px",
        fontFamily: "system-ui, sans-serif",
      }}
    >
      <h1 style={{ fontSize: 24, marginBottom: 4 }}>
        SNIP-36 Proving Playground
      </h1>
      <p style={{ color: "#666", marginBottom: 24, fontSize: 14 }}>
        Deploy a counter, then prove state transitions off-chain and submit them
        via RPC.
      </p>

      <CounterDisplay
        value={state.counterValue}
        contractAddress={state.contractAddress}
        loading={proving}
      />

      {state.error && (
        <div style={errorStyle}>
          {state.error}
        </div>
      )}

      {/* Step 1: Generate Key */}
      <StepCard number={1} title="Generate Stark Key Pair" status={steps[1]}>
        {!state.keyPair ? (
          <button onClick={handleGenerateKey} style={btnStyle}>
            Generate Key Pair
          </button>
        ) : (
          <div
            style={{ fontFamily: "monospace", fontSize: 13, lineHeight: 2 }}
          >
            <div>
              <strong>Private key:</strong>{" "}
              {truncateHex(state.keyPair.privateKey, 8)}
            </div>
            <div>
              <strong>Public key:</strong>{" "}
              {truncateHex(state.keyPair.publicKey, 8)}
            </div>
            <div>
              <strong>Account address:</strong>{" "}
              {truncateHex(state.keyPair.accountAddress, 8)}
            </div>
          </div>
        )}
        <Explainer title="How Stark keys work">
          <p>
            Starknet uses the STARK-friendly elliptic curve for signatures. A
            random 252-bit private key is generated in your browser. The public
            key is derived from it, and the account address is computed as a hash
            of the OpenZeppelin Account contract class hash + the public key.
          </p>
        </Explainer>
      </StepCard>

      {/* Step 2: Fund Account */}
      <StepCard number={2} title="Fund Account with STRK" status={steps[2]}>
        <p style={descStyle}>
          The backend's master account will transfer 0.01 STRK to your generated
          address.
        </p>
        <button onClick={handleFund} disabled={loading} style={btnStyle}>
          {loading && steps[2] === "active" ? "Funding..." : "Fund Account"}
        </button>
        {state.fundTxHash && (
          <div style={txStyle}>tx: {truncateHex(state.fundTxHash)}</div>
        )}
        <Explainer title="Why funding is needed">
          <p>
            On Starknet, every transaction requires gas fees paid in STRK
            tokens. Before your account can do anything on-chain, it needs a
            balance.
          </p>
        </Explainer>
      </StepCard>

      {/* Step 3: Deploy Account */}
      <StepCard
        number={3}
        title="Deploy Account Contract"
        status={steps[3]}
      >
        <p style={descStyle}>
          Deploy an OpenZeppelin account contract tied to your key.
        </p>
        <button
          onClick={handleDeployAccount}
          disabled={loading}
          style={btnStyle}
        >
          {loading && steps[3] === "active"
            ? "Deploying..."
            : "Deploy Account"}
        </button>
        <Explainer title="Account abstraction on Starknet">
          <p>
            Unlike Ethereum, Starknet accounts are smart contracts. The
            OpenZeppelin Account contract validates signatures using your Stark
            public key.
          </p>
        </Explainer>
      </StepCard>

      {/* Step 4: Deploy Counter */}
      <StepCard number={4} title="Deploy Counter Contract" status={steps[4]}>
        <p style={descStyle}>
          A simple Cairo contract with <code>increment(amount)</code> and{" "}
          <code>get_counter()</code>.
        </p>
        <button
          onClick={handleDeployCounter}
          disabled={loading}
          style={btnStyle}
        >
          {loading && steps[4] === "active"
            ? "Deploying..."
            : "Deploy Counter"}
        </button>
        {state.classHash && (
          <div style={txStyle}>class: {truncateHex(state.classHash)}</div>
        )}
        {state.contractAddress && (
          <div style={txStyle}>
            contract: {truncateHex(state.contractAddress)}
          </div>
        )}
        <Explainer title="How contract deployment works">
          <p>
            First the Counter's compiled Cairo bytecode is{" "}
            <strong>declared</strong> (registered as a class). Then a new
            instance is <strong>deployed</strong> with a unique salt, giving it a
            deterministic address. The counter starts at 0.
          </p>
        </Explainer>
      </StepCard>

      {/* Step 5: SNOS Virtual Blocks */}
      <StepCard
        number={5}
        title="Prove & Submit SNOS Blocks"
        status={steps[5]}
      >
        <p style={descStyle}>
          Construct a transaction <strong>off-chain</strong>, prove it in the
          virtual OS with stwo, and submit the proof via RPC. The
          counter updates on-chain without a standard invoke.
        </p>

        {/* Configuration */}
        <div
          style={{
            display: "flex",
            gap: 16,
            marginBottom: 16,
            flexWrap: "wrap",
          }}
        >
          <label style={labelStyle}>
            <span>Increment amount</span>
            <input
              type="number"
              min={1}
              max={1000}
              value={incrementAmount}
              onChange={(e) =>
                setIncrementAmount(Math.max(1, parseInt(e.target.value) || 1))
              }
              disabled={proving}
              style={inputStyle}
            />
          </label>
          <label style={labelStyle}>
            <span>Calls per block</span>
            <input
              type="number"
              min={1}
              max={10}
              value={incrementsPerBlock}
              onChange={(e) =>
                setIncrementsPerBlock(
                  Math.max(1, parseInt(e.target.value) || 1)
                )
              }
              disabled={proving}
              style={inputStyle}
            />
          </label>
          <div
            style={{
              display: "flex",
              alignItems: "flex-end",
              fontSize: 13,
              color: "#666",
              paddingBottom: 6,
            }}
          >
            = +{expectedIncrement} per block
          </div>
        </div>

        <button
          onClick={handleProveBlock}
          disabled={proving}
          style={{
            ...btnStyle,
            background: proving ? "#999" : "#7c3aed",
            width: "100%",
            padding: "12px 20px",
            fontSize: 15,
          }}
        >
          {proving
            ? phaseLabel(state.provePhase)
            : `Prove & Submit (+${expectedIncrement})`}
        </button>

        <LogPanel logs={state.proveLogs} />

        {/* Block History */}
        {state.blockHistory.length > 0 && (
          <div style={{ marginTop: 16 }}>
            <div
              style={{
                fontSize: 13,
                fontWeight: 600,
                marginBottom: 8,
                color: "#333",
              }}
            >
              Completed Blocks
            </div>
            <table
              style={{
                width: "100%",
                fontSize: 12,
                fontFamily: "monospace",
                borderCollapse: "collapse",
              }}
            >
              <thead>
                <tr style={{ borderBottom: "1px solid #ddd" }}>
                  <th style={thStyle}>#</th>
                  <th style={thStyle}>Counter</th>
                  <th style={thStyle}>Proof</th>
                  <th style={thStyle}>Time</th>
                  <th style={thStyle}>Tx</th>
                </tr>
              </thead>
              <tbody>
                {state.blockHistory.map((b) => (
                  <tr
                    key={b.blockIndex}
                    style={{ borderBottom: "1px solid #eee" }}
                  >
                    <td style={tdStyle}>{b.blockIndex}</td>
                    <td style={tdStyle}>
                      {b.counterBefore} &rarr; {b.counterAfter}{" "}
                      <span style={{ color: "#22c55e" }}>
                        (+{b.increment})
                      </span>
                    </td>
                    <td style={tdStyle}>
                      {(b.proofSize / 1024).toFixed(0)} KB
                    </td>
                    <td style={tdStyle}>{(b.durationMs / 1000).toFixed(1)}s</td>
                    <td style={tdStyle}>{truncateHex(b.txHash, 4)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}

        <Explainer title="How SNIP-36 proving works">
          <p>
            <strong>1. Construct:</strong> The transaction is built off-chain
            (not submitted to the sequencer). The server's account signs it for
            the virtual OS's <code>__validate__</code>.
          </p>
          <p>
            <strong>2. Prove:</strong> The virtual OS executes the transaction
            against on-chain state, producing an execution trace. The stwo prover
            generates a STARK proof from this trace.
          </p>
          <p>
            <strong>3. Submit:</strong> A proof-bearing transaction (with{" "}
            <code>proof_facts</code>) is signed and submitted to the privacy
            Starknet node, which verifies the proof on-chain.
          </p>
          <p>
            <strong>4. Verify:</strong> Once included on-chain, the counter
            reflects the proven state transition.
          </p>
        </Explainer>
      </StepCard>

      {/* Refresh button */}
      {state.contractAddress && (
        <div style={{ textAlign: "center", marginTop: 16 }}>
          <button
            onClick={refreshCounter}
            style={{ ...btnStyle, background: "#666" }}
          >
            Refresh Counter Value
          </button>
        </div>
      )}
    </div>
  );
}

const btnStyle: React.CSSProperties = {
  padding: "10px 20px",
  background: "#4a9eff",
  color: "white",
  border: "none",
  borderRadius: 6,
  cursor: "pointer",
  fontSize: 14,
  fontWeight: 600,
};

const txStyle: React.CSSProperties = {
  marginTop: 8,
  fontFamily: "monospace",
  fontSize: 12,
  color: "#666",
};

const descStyle: React.CSSProperties = {
  fontSize: 13,
  color: "#666",
  margin: "0 0 12px",
};

const errorStyle: React.CSSProperties = {
  padding: 12,
  background: "#fef2f2",
  border: "1px solid #fca5a5",
  borderRadius: 6,
  color: "#dc2626",
  marginBottom: 16,
  fontSize: 13,
};

const labelStyle: React.CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 4,
  fontSize: 13,
  color: "#333",
  fontWeight: 500,
};

const inputStyle: React.CSSProperties = {
  width: 80,
  padding: "6px 8px",
  border: "1px solid #ddd",
  borderRadius: 4,
  fontSize: 14,
  fontFamily: "monospace",
};

const thStyle: React.CSSProperties = {
  textAlign: "left",
  padding: "6px 8px",
  fontSize: 11,
  color: "#999",
  fontWeight: 600,
  textTransform: "uppercase",
};

const tdStyle: React.CSSProperties = {
  padding: "6px 8px",
};
