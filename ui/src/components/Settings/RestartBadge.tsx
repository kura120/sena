import "./RestartBadge.css";

interface RestartBadgeProps {
  subsystem?: string;
}

export function RestartBadge({ subsystem }: RestartBadgeProps) {
  return (
    <span className="restart-badge" title={subsystem ? `Restart ${subsystem} to apply` : "Restart required"}>
      ↺ Restart Required
    </span>
  );
}
