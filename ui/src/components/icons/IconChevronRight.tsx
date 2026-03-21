interface IconProps {
  size?: number;
  className?: string;
}

export function IconChevronRight({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5} className={className}>
      <path d="M6 4L10 8L6 12" />
    </svg>
  );
}
