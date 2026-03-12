/**
 * API client for the SNIP-36 Playground backend.
 */

const API_BASE = "/api";

async function post<T>(path: string, body: Record<string, unknown>): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({ detail: res.statusText }));
    throw new Error(err.detail || `API error ${res.status}`);
  }
  return res.json();
}

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`);
  if (!res.ok) {
    const err = await res.json().catch(() => ({ detail: res.statusText }));
    throw new Error(err.detail || `API error ${res.status}`);
  }
  return res.json();
}

export const api = {
  health: () => get<{ status: string; rpc_url: string }>("/health"),

  fund: (accountAddress: string) =>
    post<{ tx_hash: string; amount: string }>("/fund", {
      account_address: accountAddress,
    }),

  deployAccount: (sessionId: string, publicKey: string, accountAddress: string) =>
    post<{ account_address: string; tx_hash: string | null; block_number: number | null }>("/deploy-account", {
      session_id: sessionId,
      public_key: publicKey,
      account_address: accountAddress,
    }),

  deployCounter: (sessionId: string) =>
    post<{ class_hash: string; contract_address: string; tx_hash: string | null; block_number: number | null }>(
      "/deploy-counter",
      { session_id: sessionId }
    ),

  readCounter: (contractAddress: string) =>
    post<{ counter_value: number }>("/read-counter", {
      contract_address: contractAddress,
    }),

  getNonce: (accountAddress: string) =>
    get<{ nonce: number; nonce_hex: string }>(`/nonce/${accountAddress}`),

  invoke: (params: {
    sessionId: string;
    amount: number;
    signatureR: string;
    signatureS: string;
    nonce: number;
  }) =>
    post<{ tx_hash: string }>("/invoke", {
      session_id: params.sessionId,
      amount: params.amount,
      signature_r: params.signatureR,
      signature_s: params.signatureS,
      nonce: params.nonce,
    }),

  /** Returns an EventSource for SSE streaming of proof logs (legacy). */
  proveStream: (sessionId: string): EventSource =>
    new EventSource(`${API_BASE}/prove/${sessionId}`),

  /** Returns an EventSource for the full SNIP-36 prove-and-submit cycle. */
  proveBlock: (
    sessionId: string,
    incrementAmount: number,
    incrementsPerBlock: number,
  ): EventSource =>
    new EventSource(
      `${API_BASE}/prove-block/${sessionId}?increment_amount=${incrementAmount}&increments_per_block=${incrementsPerBlock}`
    ),

  submitProof: (sessionId: string) =>
    post<{ tx_hash?: string; output: string }>("/submit-proof", {
      session_id: sessionId,
    }),
};
