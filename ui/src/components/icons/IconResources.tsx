interface IconProps {
  size?: number;
  className?: string;
}

export function IconResources({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <rect x="2" y="10" width="3" height="4" rx="0.5" fill="currentColor" opacity="0.6" />
      <rect x="6.5" y="6" width="3" height="8" rx="0.5" fill="currentColor" opacity="0.8" />
      <rect x="11" y="2" width="3" height="12" rx="0.5" fill="currentColor" />
    </svg>
  );
}
