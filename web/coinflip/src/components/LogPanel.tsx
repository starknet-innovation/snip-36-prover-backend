import { useEffect, useRef } from "react";

export default function LogPanel({ logs }: { logs: string[] }) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (ref.current) ref.current.scrollTop = ref.current.scrollHeight;
  }, [logs]);

  if (logs.length === 0) return null;

  return (
    <div
      ref={ref}
      style={{
        background: "#0d1117",
        border: "1px solid #21262d",
        borderRadius: 8,
        padding: 12,
        maxHeight: 200,
        overflowY: "auto",
        fontFamily: "monospace",
        fontSize: 12,
        lineHeight: 1.6,
        color: "#7ee787",
        marginTop: 16,
      }}
    >
      {logs.map((line, i) => (
        <div key={i}>{line}</div>
      ))}
    </div>
  );
}
