interface IconProps {
  size?: number;
  className?: string;
}

export function IconSubsystemHealth({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5} className={className}>
      <path d="M2 8H4.5L6 3L10 13L11.5 8H14" />
    </svg>
  );
}
