import { AlertTriangle, CheckCircle2, Info, XCircle } from "lucide-react";

interface ErrorDisplayProps {
  title: string;
  detail?: string;
  tone?: "error" | "warning" | "info" | "success";
}

const icons = {
  error: XCircle,
  warning: AlertTriangle,
  info: Info,
  success: CheckCircle2,
};

export function ErrorDisplay({ title, detail, tone = "error" }: ErrorDisplayProps) {
  const Icon = icons[tone];
  return (
    <div className={`notice notice-${tone}`} role={tone === "error" ? "alert" : "status"}>
      <Icon size={18} strokeWidth={1.6} />
      <div>
        <div className="notice-title">{title}</div>
        {detail ? <div className="notice-detail">{detail}</div> : null}
      </div>
    </div>
  );
}
