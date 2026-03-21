interface IconProps {
  size?: number;
  className?: string;
}

export function IconBootTimeline({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5} className={className}>
      <circle cx="8" cy="8" r="6" />
      <path d="M8 4V8L11 10" />
    </svg>
  );
}
