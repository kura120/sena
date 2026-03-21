interface IconProps {
  size?: number;
  className?: string;
}

export function IconEventBus({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5} className={className}>
      <path d="M2.5 4.5H13.5" />
      <path d="M2.5 8H13.5" />
      <path d="M2.5 11.5H13.5" />
    </svg>
  );
}
