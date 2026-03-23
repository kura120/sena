import type { IconProps } from "../../types";

export function IconFolder({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" className={className}>
      <path d="M1.5 3a.5.5 0 0 1 .5-.5h4.586a.5.5 0 0 1 .353.146L8.354 4.06a.5.5 0 0 0 .353.147H14a.5.5 0 0 1 .5.5v8a.5.5 0 0 1-.5.5H2a.5.5 0 0 1-.5-.5V3Z" stroke="currentColor" strokeWidth="1.2"/>
    </svg>
  );
}
