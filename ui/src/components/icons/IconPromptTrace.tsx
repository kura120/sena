interface IconProps {
  size?: number;
  className?: string;
}

export function IconPromptTrace({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <path d="M3 3h10M3 6.5h7M3 10h10M3 13.5h5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );
}
