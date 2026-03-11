import { useEffect, useRef } from "react";

interface Props {
  logs: string[];
}

export function LogPanel({ logs }: Props) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (ref.current) {
      ref.current.scrollTop = ref.current.scrollHeight;
    }
  }, [logs]);

  if (logs.length === 0) return null;

  return (
    <div
      ref={ref}
      style={{
        marginTop: 12,
        padding: 12,
        background: "#1a1a2e",
        color: "#0f0",
        fontFamily: "monospace",
        fontSize: 12,
        borderRadius: 6,
        maxHeight: 200,
        overflow: "auto",
        whiteSpace: "pre-wrap",
        wordBreak: "break-all",
      }}
    >
      {logs.map((line, i) => (
        <div key={i}>{line}</div>
      ))}
    </div>
  );
}
