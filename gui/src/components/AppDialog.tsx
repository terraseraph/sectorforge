import { useState } from "react";
import { type DialogRequest, useDialog } from "../store/dialog";
import { Button, TextInput } from "./ui";

export function AppDialog() {
  const req = useDialog((s) => s.req);
  return req && <Ask key={req.id} req={req} />;
}

function Ask({ req }: { req: DialogRequest }) {
  const [value, setValue] = useState(req.input ?? "");

  const asking = req.input !== undefined;
  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && req.resolve(null)}>
      <form
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="dlg-title"
        onSubmit={(e) => {
          e.preventDefault();
          req.resolve(asking ? value : "");
        }}
        onKeyDown={(e) => {
          if (e.key !== "Escape") return;
          // Close only the dialog, not the console or inspector underneath.
          e.stopPropagation();
          req.resolve(null);
        }}
      >
        <h3 id="dlg-title">{req.title}</h3>
        {req.message && <p className="dialog-msg">{req.message}</p>}
        {asking && <TextInput autoFocus aria-labelledby="dlg-title" value={value} onChange={(e) => setValue(e.target.value)} />}
        <div className="actions">
          <Button onClick={() => req.resolve(null)}>Cancel</Button>
          <Button variant="primary" type="submit" autoFocus={!asking}>
            {req.okLabel}
          </Button>
        </div>
      </form>
    </div>
  );
}
