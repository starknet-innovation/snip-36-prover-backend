interface Props {
  number: number;
  title: string;
  status: "pending" | "active" | "done" | "error";
  children: React.ReactNode;
}

const statusColors = {
  pending: "#ddd",
  active: "#4a9eff",
  done: "#22c55e",
  error: "#ef4444",
};

const statusLabels = {
  pending: "",
  active: "In progress...",
  done: "Done",
  error: "Error",
};

export function StepCard({ number, title, status, children }: Props) {
  return (
    <div
      style={{
        border: `2px solid ${statusColors[status]}`,
        borderRadius: 8,
        padding: 20,
        marginBottom: 16,
        opacity: status === "pending" ? 0.5 : 1,
        transition: "all 0.3s",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 12 }}>
        <div
          style={{
            width: 32,
            height: 32,
            borderRadius: "50%",
            background: statusColors[status],
            color: status === "pending" ? "#999" : "white",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            fontWeight: 700,
            fontSize: 14,
          }}
        >
          {status === "done" ? "\u2713" : number}
        </div>
        <h3 style={{ margin: 0, fontSize: 16 }}>{title}</h3>
        {statusLabels[status] && (
          <span style={{ fontSize: 12, color: statusColors[status], marginLeft: "auto" }}>
            {statusLabels[status]}
          </span>
        )}
      </div>
      {status !== "pending" && <div>{children}</div>}
    </div>
  );
}
