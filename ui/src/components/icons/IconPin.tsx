interface IconProps {
  size?: number;
  className?: string;
}

export function IconPin({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5} className={className}>
      <path d="M8 2.5V9.5M8 9.5L5.5 12.5H10.5L8 9.5Z" />
      <path d="M10.5 12.5L8 14.5L5.5 12.5" />
      <circle cx="8" cy="3" r="1.5" />
    </svg>
  );
}
