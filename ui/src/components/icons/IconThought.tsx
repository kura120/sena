import type { IconProps } from "../../types";

export function IconThought({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <path
        d="M8 2a4 4 0 0 0-4 4c0 1.5.8 2.7 2 3.4V11a1 1 0 0 0 1 1h2a1 1 0 0 0 1-1V9.4c1.2-.7 2-1.9 2-3.4a4 4 0 0 0-4-4Z"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinejoin="round"
      />
      <path d="M6.5 13h3M7 14.5h2" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
    </svg>
  );
}
