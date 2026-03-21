interface IconProps {
  size?: number;
  className?: string;
}

export function IconChat({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.5} className={className}>
      <path d="M2.5 2.5H13.5V11.5H8.5L5.5 14.5V11.5H2.5V2.5Z" />
    </svg>
  );
}
