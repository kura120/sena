interface IconProps {
  size?: number;
  className?: string;
}

export function IconMemoryStats({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <rect x="2" y="2" width="12" height="12" rx="2" stroke="currentColor" strokeWidth="1.5" />
      <line x1="5" y1="6" x2="11" y2="6" stroke="currentColor" strokeWidth="1.5" opacity="0.6" />
      <line x1="5" y1="8.5" x2="11" y2="8.5" stroke="currentColor" strokeWidth="1.5" opacity="0.8" />
      <line x1="5" y1="11" x2="11" y2="11" stroke="currentColor" strokeWidth="1.5" />
    </svg>
  );
}
