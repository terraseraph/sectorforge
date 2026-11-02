import { create } from "zustand";

/** One question shown by <AppDialog>. `input` set = ask for text (pre-filled with it); absent = a yes/no question. */
export interface DialogRequest {
  /** New for every question, so the dialog starts fresh. */
  id: number;
  title: string;
  message?: string;
  okLabel: string;
  input?: string;
  /** The text (or "" for a yes/no question) on OK; null on Cancel. */
  resolve: (value: string | null) => void;
}

export const useDialog = create<{ req: DialogRequest | null }>(() => ({ req: null }));

let asked = 0;

/** In-app replacement for confirm()/prompt(), which some webviews block or answer "Cancel" without showing. */
export function showDialog(opts: Omit<DialogRequest, "id" | "resolve">): Promise<string | null> {
  // A newer question replaces one nobody answered.
  useDialog.getState().req?.resolve(null);
  return new Promise((resolve) =>
    useDialog.setState({
      req: {
        ...opts,
        id: ++asked,
        resolve: (value) => {
          useDialog.setState({ req: null });
          resolve(value);
        },
      },
    }),
  );
}
