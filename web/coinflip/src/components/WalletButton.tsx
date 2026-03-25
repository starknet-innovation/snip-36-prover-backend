interface WalletButtonProps {
  address: string | null;
  onConnect: () => void;
  onDisconnect: () => void;
}

function truncate(hex: string, chars = 6): string {
  if (hex.length <= chars * 2 + 4) return hex;
  return `${hex.slice(0, chars + 2)}...${hex.slice(-chars)}`;
}

export default function WalletButton({
  address,
  onConnect,
  onDisconnect,
}: WalletButtonProps) {
  if (address) {
    return (
      <button
        onClick={onDisconnect}
        style={{
          background: "rgba(255,255,255,0.1)",
          border: "1px solid rgba(255,255,255,0.2)",
          color: "#e0e0e0",
          padding: "8px 16px",
          borderRadius: 8,
          cursor: "pointer",
          fontFamily: "monospace",
          fontSize: 14,
        }}
      >
        {truncate(address)}
      </button>
    );
  }

  return (
    <button
      onClick={onConnect}
      style={{
        background: "linear-gradient(135deg, #ffd700, #f0a000)",
        border: "none",
        color: "#1a1a2e",
        padding: "12px 24px",
        borderRadius: 8,
        cursor: "pointer",
        fontSize: 16,
        fontWeight: 700,
      }}
    >
      Connect Wallet
    </button>
  );
}
