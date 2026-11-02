import { useApp } from "./app";
import { useRecover } from "./recover";

/**
 * The Recover source as typed: the Image destination until the user types their own.
 * Untrimmed, so it can back an input (trimming there would eat the space in "My Images").
 */
export function useRecoverSourceInput(): string {
  const dest = useApp((s) => s.dest);
  const source = useRecover((s) => s.source);
  const edited = useRecover((s) => s.sourceEdited);
  return edited ? source : dest;
}

/** The Recover source as a path to use. */
export const useRecoverSource = () => useRecoverSourceInput().trim();
