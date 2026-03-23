interface IconProps {
  size?: number;
  className?: string;
}

export function IconThoughtStream({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <circle cx="8" cy="8" r="3" stroke="currentColor" strokeWidth="1.5" />
      <circle cx="8" cy="8" r="6" stroke="currentColor" strokeWidth="1" opacity="0.4" />
      <circle cx="8" cy="8" r="1" fill="currentColor" />
    </svg>
  );
}
