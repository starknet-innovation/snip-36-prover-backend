interface Props {
  value: number | null;
  contractAddress: string | null;
  loading?: boolean;
}

export function CounterDisplay({ value, contractAddress, loading }: Props) {
  return (
    <div
      style={{
        textAlign: "center",
        padding: 24,
        background: "linear-gradient(135deg, #667eea 0%, #764ba2 100%)",
        borderRadius: 12,
        color: "white",
        marginBottom: 24,
      }}
    >
      <div style={{ fontSize: 14, opacity: 0.8, marginBottom: 4 }}>
        Counter Value
      </div>
      <div style={{ fontSize: 64, fontWeight: 700, fontFamily: "monospace" }}>
        {loading ? "..." : value ?? "-"}
      </div>
      {contractAddress && (
        <div style={{ fontSize: 11, opacity: 0.6, marginTop: 8, fontFamily: "monospace" }}>
          {contractAddress}
        </div>
      )}
    </div>
  );
}
