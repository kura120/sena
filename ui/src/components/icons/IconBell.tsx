import type { IconProps } from "../../types";

export function IconBell({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <path
        d="M8 1.5a4 4 0 0 0-4 4v2.5l-1 2h10l-1-2V5.5a4 4 0 0 0-4-4Z"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinejoin="round"
      />
      <path
        d="M6.5 12.5a1.5 1.5 0 0 0 3 0"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinecap="round"
      />
    </svg>
  );
}
