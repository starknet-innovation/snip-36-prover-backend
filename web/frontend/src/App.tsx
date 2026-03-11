import { useState, useCallback } from "react";
import {
  generateKeyPair,
  signTransaction,
  computeInvokeV3TxHash,
  truncateHex,
  SELECTORS,
  type StarkKeyPair,
} from "./lib/starknet";
import { api } from "./lib/api";
import { StepCard } from "./components/StepCard";
import { CounterDisplay } from "./components/CounterDisplay";
import { LogPanel } from "./components/LogPanel";
import { Explainer } from "./components/Explainer";

type StepStatus = "pending" | "active" | "done" | "error";

interface AppState {
  sessionId: string;
  keyPair: StarkKeyPair | null;
  fundTxHash: string | null;
  accountDeployed: boolean;
  contractAddress: string | null;
  classHash: string | null;
  counterValue: number | null;
  invokeTxHash: string | null;
  proofSize: number | null;
  proofSubmitTxHash: string | null;
  proveLogs: string[];
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
  invokeTxHash: null,
  proofSize: null,
  proofSubmitTxHash: null,
  proveLogs: [],
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
    6: "pending",
    7: "pending",
  });
  const [loading, setLoading] = useState(false);

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

  // ── Step 5: Invoke Increment ──────────────────────────

  const handleInvoke = useCallback(async () => {
    if (!state.keyPair || !state.contractAddress) return;
    setLoading(true);
    setError("");
    try {
      // Get nonce
      const { nonce } = await api.getNonce(state.keyPair.accountAddress);

      // Build multicall calldata: [num_calls, to, selector, calldata_len, ...calldata]
      const calldata = [
        "0x1",
        state.contractAddress,
        SELECTORS.increment,
        "0x1",
        "0x1",
      ];

      // Compute invoke v3 tx hash client-side using Poseidon
      const txHash = computeInvokeV3TxHash({
        senderAddress: state.keyPair.accountAddress,
        calldata,
        nonce,
      });

      // Sign in browser with the generated private key
      const sig = signTransaction(txHash, state.keyPair.privateKey);

      const result = await api.invoke({
        sessionId: state.sessionId,
        amount: 1,
        signatureR: sig.r,
        signatureS: sig.s,
        nonce,
      });

      setState((s) => ({ ...s, invokeTxHash: result.tx_hash }));

      // Poll counter value
      setTimeout(async () => {
        if (state.contractAddress) {
          try {
            const { counter_value } = await api.readCounter(state.contractAddress);
            setState((s) => ({ ...s, counterValue: counter_value }));
          } catch {}
        }
      }, 15000);

      setStep(5, "done");
      setStep(6, "active");
    } catch (e: any) {
      setError(e.message);
      setStep(5, "error");
    }
    setLoading(false);
  }, [state.keyPair, state.contractAddress, state.sessionId]);

  // ── Step 6: Prove ─────────────────────────────────────

  const handleProve = useCallback(async () => {
    setStep(6, "active");
    setState((s) => ({ ...s, proveLogs: [], error: null }));

    const source = api.proveStream(state.sessionId);

    source.addEventListener("log", (e: MessageEvent) => {
      setState((s) => ({ ...s, proveLogs: [...s.proveLogs, e.data] }));
    });

    source.addEventListener("complete", (e: MessageEvent) => {
      source.close();
      const data = JSON.parse(e.data);
      setState((s) => ({ ...s, proofSize: data.proof_size }));
      setStep(6, "done");
      setStep(7, "active");
    });

    source.addEventListener("error", (e: MessageEvent) => {
      source.close();
      setError(e.data || "Proof generation failed");
      setStep(6, "error");
    });

    source.onerror = () => {
      source.close();
    };
  }, [state.sessionId]);

  // ── Step 7: Submit Proof ──────────────────────────────

  const handleSubmitProof = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const result = await api.submitProof(state.sessionId);
      setState((s) => ({ ...s, proofSubmitTxHash: result.tx_hash || null }));
      setStep(7, "done");

      // Refresh counter
      if (state.contractAddress) {
        setTimeout(async () => {
          try {
            const { counter_value } = await api.readCounter(state.contractAddress!);
            setState((s) => ({ ...s, counterValue: counter_value }));
          } catch {}
        }, 15000);
      }
    } catch (e: any) {
      setError(e.message);
      setStep(7, "error");
    }
    setLoading(false);
  }, [state.sessionId, state.contractAddress]);

  // ── Refresh counter ───────────────────────────────────

  const refreshCounter = useCallback(async () => {
    if (!state.contractAddress) return;
    try {
      const { counter_value } = await api.readCounter(state.contractAddress);
      setState((s) => ({ ...s, counterValue: counter_value }));
    } catch {}
  }, [state.contractAddress]);

  // ── Render ────────────────────────────────────────────

  return (
    <div style={{ maxWidth: 720, margin: "0 auto", padding: "32px 16px", fontFamily: "system-ui, sans-serif" }}>
      <h1 style={{ fontSize: 24, marginBottom: 4 }}>SNIP-36 Proving Playground</h1>
      <p style={{ color: "#666", marginBottom: 24, fontSize: 14 }}>
        Generate a key, deploy a counter, increment it, then prove the transaction with stwo.
      </p>

      <CounterDisplay
        value={state.counterValue}
        contractAddress={state.contractAddress}
        loading={loading && steps[5] === "active"}
      />

      {state.error && (
        <div
          style={{
            padding: 12,
            background: "#fef2f2",
            border: "1px solid #fca5a5",
            borderRadius: 6,
            color: "#dc2626",
            marginBottom: 16,
            fontSize: 13,
          }}
        >
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
          <div style={{ fontFamily: "monospace", fontSize: 13, lineHeight: 2 }}>
            <div><strong>Private key:</strong> {truncateHex(state.keyPair.privateKey, 8)}</div>
            <div><strong>Public key:</strong> {truncateHex(state.keyPair.publicKey, 8)}</div>
            <div><strong>Account address:</strong> {truncateHex(state.keyPair.accountAddress, 8)}</div>
          </div>
        )}
        <Explainer title="How Stark keys work">
          <p>
            Starknet uses the STARK-friendly elliptic curve for signatures. A random 252-bit
            private key is generated in your browser. The public key is derived from it, and
            the account address is computed as a hash of the OpenZeppelin Account contract class
            hash + the public key (used as both salt and constructor argument).
          </p>
          <p>
            This key never leaves your browser. The backend only sees the public key and address.
          </p>
        </Explainer>
      </StepCard>

      {/* Step 2: Fund Account */}
      <StepCard number={2} title="Fund Account with STRK" status={steps[2]}>
        <p style={{ fontSize: 13, color: "#666", margin: "0 0 12px" }}>
          The backend's master account will transfer 10 STRK to your generated address.
        </p>
        <button onClick={handleFund} disabled={loading} style={btnStyle}>
          {loading && steps[2] === "active" ? "Funding..." : "Fund Account"}
        </button>
        {state.fundTxHash && (
          <div style={txStyle}>tx: {truncateHex(state.fundTxHash)}</div>
        )}
        <Explainer title="Why funding is needed">
          <p>
            On Starknet, every transaction requires gas fees paid in STRK tokens.
            Before your account can do anything on-chain, it needs a balance.
            On integration/testnet, we use a pre-funded master account to bootstrap new accounts.
          </p>
        </Explainer>
      </StepCard>

      {/* Step 3: Deploy Account */}
      <StepCard number={3} title="Deploy Account Contract" status={steps[3]}>
        <p style={{ fontSize: 13, color: "#666", margin: "0 0 12px" }}>
          Deploy an OpenZeppelin account contract tied to your key.
        </p>
        <button onClick={handleDeployAccount} disabled={loading} style={btnStyle}>
          {loading && steps[3] === "active" ? "Deploying..." : "Deploy Account"}
        </button>
        <Explainer title="Account abstraction on Starknet">
          <p>
            Unlike Ethereum, Starknet accounts are smart contracts. The OpenZeppelin Account
            contract validates signatures using your Stark public key. Deploying it makes your
            generated address a real, usable account on-chain.
          </p>
        </Explainer>
      </StepCard>

      {/* Step 4: Deploy Counter */}
      <StepCard number={4} title="Deploy Counter Contract" status={steps[4]}>
        <p style={{ fontSize: 13, color: "#666", margin: "0 0 12px" }}>
          A simple Cairo contract with <code>increment(amount)</code> and <code>get_counter()</code>.
        </p>
        <button onClick={handleDeployCounter} disabled={loading} style={btnStyle}>
          {loading && steps[4] === "active" ? "Deploying..." : "Deploy Counter"}
        </button>
        {state.classHash && (
          <div style={txStyle}>class: {truncateHex(state.classHash)}</div>
        )}
        {state.contractAddress && (
          <div style={txStyle}>contract: {truncateHex(state.contractAddress)}</div>
        )}
        <Explainer title="How contract deployment works">
          <p>
            First the Counter's compiled Cairo bytecode is <strong>declared</strong> (registered
            as a class). Then a new instance is <strong>deployed</strong> with a unique salt,
            giving it a deterministic address. The counter starts at 0.
          </p>
        </Explainer>
      </StepCard>

      {/* Step 5: Invoke Increment */}
      <StepCard number={5} title="Increment Counter (Normal Invoke)" status={steps[5]}>
        <p style={{ fontSize: 13, color: "#666", margin: "0 0 12px" }}>
          Sign an <code>increment(1)</code> transaction with your browser-generated key.
        </p>
        <button onClick={handleInvoke} disabled={loading} style={btnStyle}>
          {loading && steps[5] === "active" ? "Signing & submitting..." : "Increment +1"}
        </button>
        {state.invokeTxHash && (
          <div style={txStyle}>tx: {truncateHex(state.invokeTxHash)}</div>
        )}
        <Explainer title="Transaction signing">
          <p>
            The transaction hash is computed over all fields (sender, calldata, nonce, fees, etc.)
            using Poseidon hash. Your private key signs this hash using the STARK curve.
            The account contract on-chain verifies the signature before executing the call.
          </p>
        </Explainer>
      </StepCard>

      {/* Step 6: Prove */}
      <StepCard number={6} title="Prove the Transaction (stwo)" status={steps[6]}>
        <p style={{ fontSize: 13, color: "#666", margin: "0 0 12px" }}>
          Re-execute the transaction in the virtual OS and generate a STARK proof with stwo.
          This takes a few minutes.
        </p>
        <button onClick={handleProve} disabled={loading || steps[6] === "done"} style={btnStyle}>
          {steps[6] === "active" && state.proveLogs.length > 0
            ? "Proving..."
            : "Start Proving"}
        </button>
        <LogPanel logs={state.proveLogs} />
        {state.proofSize && (
          <div style={txStyle}>
            Proof generated: {(state.proofSize / 1024).toFixed(1)} KB
          </div>
        )}
        <Explainer title="How SNIP-36 proving works">
          <p>
            <strong>Phase 1 (Virtual OS):</strong> The transaction is re-executed inside
            a stripped-down Starknet OS. This produces a <em>Cairo PIE</em> — an execution
            trace capturing every computation step.
          </p>
          <p>
            <strong>Phase 2 (stwo prover):</strong> The PIE is fed through a bootloader
            into the stwo-cairo prover, which generates a STARK proof. This proof
            cryptographically attests that the transaction was executed correctly — without
            revealing the full execution trace.
          </p>
        </Explainer>
      </StepCard>

      {/* Step 7: Submit Proof */}
      <StepCard number={7} title="Submit Proof-Bearing Transaction" status={steps[7]}>
        <p style={{ fontSize: 13, color: "#666", margin: "0 0 12px" }}>
          Submit an invoke transaction that includes the proof in <code>proof_facts</code>.
          The gateway verifies the proof on-chain.
        </p>
        <button onClick={handleSubmitProof} disabled={loading} style={btnStyle}>
          {loading && steps[7] === "active" ? "Submitting..." : "Submit Proof"}
        </button>
        {state.proofSubmitTxHash && (
          <div style={txStyle}>tx: {truncateHex(state.proofSubmitTxHash)}</div>
        )}
        <Explainer title="Proof-bearing transactions">
          <p>
            SNIP-36 extends <code>INVOKE_TXN_V3</code> with a <code>proof_facts</code>
            field — a list of field elements that the gateway includes in the Poseidon
            transaction hash. This means the proof is cryptographically bound to the
            transaction. Standard starknet tooling doesn't include this field, so we
            compute the hash manually.
          </p>
          <p>
            The proof verification alone costs ~75M L2 gas. This is the "virtual block"
            being anchored on-chain.
          </p>
        </Explainer>
      </StepCard>

      {/* Refresh button */}
      {state.contractAddress && (
        <div style={{ textAlign: "center", marginTop: 16 }}>
          <button onClick={refreshCounter} style={{ ...btnStyle, background: "#666" }}>
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
