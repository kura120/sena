interface IconProps {
  size?: number;
  className?: string;
}

export function IconSend({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5} className={className}>
      <path d="M14 8L2 14V8V2L14 8Z" />
      <path d="M2 8H14" />
    </svg>
  );
}
