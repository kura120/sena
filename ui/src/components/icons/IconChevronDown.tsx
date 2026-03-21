interface IconProps {
  size?: number;
  className?: string;
}

export function IconChevronDown({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5} className={className}>
      <path d="M4 6L8 10L12 6" />
    </svg>
  );
}
