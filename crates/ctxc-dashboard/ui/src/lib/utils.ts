import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** Merge class names, letting a caller's classes win over a component's. */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
