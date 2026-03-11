import { useState } from "react";

interface Props {
  title: string;
  children: React.ReactNode;
}

export function Explainer({ title, children }: Props) {
  const [open, setOpen] = useState(false);
  return (
    <div style={{ marginTop: 8, fontSize: 13, color: "#666" }}>
      <button
        onClick={() => setOpen(!open)}
        style={{
          background: "none",
          border: "none",
          color: "#4a9eff",
          cursor: "pointer",
          padding: 0,
          fontSize: 13,
        }}
      >
        {open ? "Hide" : "Learn more"}: {title}
      </button>
      {open && (
        <div
          style={{
            marginTop: 8,
            padding: 12,
            background: "#f0f4f8",
            borderRadius: 6,
            lineHeight: 1.6,
          }}
        >
          {children}
        </div>
      )}
    </div>
  );
}
