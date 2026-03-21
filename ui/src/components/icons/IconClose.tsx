interface IconProps {
  size?: number;
  className?: string;
}

export function IconClose({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5} className={className}>
      <path d="M4 4L12 12M12 4L4 12" />
    </svg>
  );
}
