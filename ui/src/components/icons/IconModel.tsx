import type { IconProps } from "../../types";

export function IconModel({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <path d="M8 1L14 4.5V11.5L8 15L2 11.5V4.5L8 1Z" stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round" />
      <path d="M8 1V8M8 8L14 4.5M8 8L2 4.5M8 8V15" stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round" />
    </svg>
  );
}
