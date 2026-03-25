const BASE = "/api";

async function post<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    throw new Error(err.detail || `HTTP ${res.status}`);
  }
  return res.json();
}

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`);
  if (!res.ok) {
    const err = await res.json().catch(() => ({}));
    throw new Error(err.detail || `HTTP ${res.status}`);
  }
  return res.json();
}

export interface CoinFlipStatus {
  deployed: boolean;
  contract_address?: string;
}

export interface DeployResponse {
  contract_address: string;
  class_hash: string;
  deploy_block: number;
}

export interface CommitResponse {
  session_id: string;
  seed_block: number;
}

export const api = {
  coinflipStatus: () => get<CoinFlipStatus>("/coinflip/status"),

  deployCoinflip: () => post<DeployResponse>("/coinflip/deploy", {}),

  commit: (commitment: string, player: string) =>
    post<CommitResponse>("/coinflip/commit", { commitment, player }),

  play: (
    sessionId: string,
    player: string,
    bet: number,
    nonce: string,
  ): EventSource =>
    new EventSource(
      `${BASE}/coinflip/play/${sessionId}?player=${encodeURIComponent(player)}&bet=${bet}&nonce=${encodeURIComponent(nonce)}`
    ),
};
