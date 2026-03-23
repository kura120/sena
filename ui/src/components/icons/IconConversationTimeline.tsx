interface IconProps {
  size?: number;
  className?: string;
}

export function IconConversationTimeline({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <line x1="4" y1="2" x2="4" y2="14" stroke="currentColor" strokeWidth="1.5" opacity="0.3" />
      <circle cx="4" cy="4" r="1.5" fill="currentColor" />
      <circle cx="4" cy="8" r="1.5" fill="currentColor" opacity="0.7" />
      <circle cx="4" cy="12" r="1.5" fill="currentColor" opacity="0.4" />
      <line x1="7" y1="4" x2="13" y2="4" stroke="currentColor" strokeWidth="1" />
      <line x1="7" y1="8" x2="13" y2="8" stroke="currentColor" strokeWidth="1" opacity="0.7" />
      <line x1="7" y1="12" x2="11" y2="12" stroke="currentColor" strokeWidth="1" opacity="0.4" />
    </svg>
  );
}
