interface CoinDisplayProps {
  side: "heads" | "tails" | null;
  spinning: boolean;
}

export default function CoinDisplay({ side, spinning }: CoinDisplayProps) {
  return (
    <div
      style={{
        width: 160,
        height: 160,
        borderRadius: "50%",
        background: spinning
          ? "linear-gradient(135deg, #ffd700, #b8860b, #ffd700)"
          : side === "heads"
          ? "linear-gradient(135deg, #ffd700 0%, #f0c040 100%)"
          : side === "tails"
          ? "linear-gradient(135deg, #c0c0c0 0%, #808080 100%)"
          : "linear-gradient(135deg, #333 0%, #555 100%)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        fontSize: 48,
        fontWeight: 700,
        color: spinning ? "#fff" : side === "heads" ? "#5a4000" : side === "tails" ? "#333" : "#888",
        boxShadow: spinning
          ? "0 0 40px rgba(255, 215, 0, 0.5)"
          : "0 4px 20px rgba(0,0,0,0.4)",
        animation: spinning ? "spin 0.6s linear infinite" : "none",
        transition: "all 0.3s ease",
        margin: "0 auto",
      }}
    >
      <style>
        {`@keyframes spin {
          0% { transform: rotateY(0deg); }
          100% { transform: rotateY(360deg); }
        }`}
      </style>
      {spinning ? "?" : side === "heads" ? "H" : side === "tails" ? "T" : "?"}
    </div>
  );
}
