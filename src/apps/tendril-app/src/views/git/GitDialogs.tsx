import React from "react";
import { bridge } from "../../api/bridge";
import { describeBridgeError } from "../../types/api";
import type { MergeMode, ResetMode } from "../../types/git";
import { openUrl } from "../../utils/opener";
import { GhostButton, PrimaryButton } from "../../components/page/kit";
import { Field, Modal, inputClass, textareaClass } from "./gitUi";

const Cancel: React.FC<{ onClose: () => void }> = ({ onClose }) => <GhostButton onClick={onClose}>Cancel</GhostButton>;

/** Submits on Enter in a single-line field. */
const onEnter = (submit: () => void) => (e: React.KeyboardEvent) => {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    submit();
  }
};

export const NewBranchDialog: React.FC<{
  from: { label: string; rev: string | undefined };
  onSubmit: (name: string, checkout: boolean) => void;
  onClose: () => void;
}> = ({ from, onSubmit, onClose }) => {
  const [name, setName] = React.useState("");
  const [checkout, setCheckout] = React.useState(true);
  const go = () => name.trim() && (onSubmit(name.trim(), checkout), onClose());
  return (
    <Modal
      title="New branch"
      onClose={onClose}
      footer={
        <>
          <Cancel onClose={onClose} />
          <PrimaryButton disabled={!name.trim()} onClick={go}>Create branch</PrimaryButton>
        </>
      }
    >
      <Field label="Name" hint="No spaces. Slashes make folders, e.g. feature/login-form.">
        <input autoFocus value={name} onChange={(e) => setName(e.target.value)} onKeyDown={onEnter(go)} placeholder="feature/my-change" className={inputClass} />
      </Field>
      <div className="text-[12.5px] text-muted-foreground">Starts from <span className="font-mono text-foreground">{from.label}</span>.</div>
      <label className="flex items-center gap-2 text-[12.5px] text-foreground">
        <input type="checkbox" checked={checkout} onChange={(e) => setCheckout(e.target.checked)} className="accent-[var(--primary)]" />
        Switch to it
      </label>
    </Modal>
  );
};

export const RenameBranchDialog: React.FC<{ name: string; onSubmit: (to: string) => void; onClose: () => void }> = ({ name, onSubmit, onClose }) => {
  const [to, setTo] = React.useState(name);
  const go = () => to.trim() && to.trim() !== name && (onSubmit(to.trim()), onClose());
  return (
    <Modal
      title={`Rename ${name}`}
      onClose={onClose}
      footer={
        <>
          <Cancel onClose={onClose} />
          <PrimaryButton disabled={!to.trim() || to.trim() === name} onClick={go}>Rename</PrimaryButton>
        </>
      }
    >
      <Field label="New name" hint="This renames the local branch only. A copy already pushed keeps its old name on the remote.">
        <input autoFocus value={to} onChange={(e) => setTo(e.target.value)} onKeyDown={onEnter(go)} className={inputClass} />
      </Field>
    </Modal>
  );
};

export const MergeDialog: React.FC<{
  current: string;
  branch: string;
  onSubmit: (mode: MergeMode) => void;
  onClose: () => void;
}> = ({ current, branch, onSubmit, onClose }) => {
  const [mode, setMode] = React.useState<MergeMode>("default");
  const options: { value: MergeMode; title: string; text: string }[] = [
    { value: "default", title: "Merge", text: "Fast-forward if it can, otherwise make a merge commit." },
    { value: "noFf", title: "Always a merge commit", text: "Keeps the branch visible in the history, even when it could fast-forward." },
    { value: "ffOnly", title: "Fast-forward only", text: "Only if nothing has diverged. Otherwise it stops and changes nothing." },
  ];
  return (
    <Modal
      title={`Merge ${branch} into ${current}`}
      onClose={onClose}
      footer={
        <>
          <Cancel onClose={onClose} />
          <PrimaryButton onClick={() => (onSubmit(mode), onClose())}>Merge</PrimaryButton>
        </>
      }
    >
      <div className="flex flex-col gap-2">
        {options.map((o) => (
          <label key={o.value} className="flex cursor-pointer gap-3 rounded-lg border border-border p-3 hover:bg-secondary/40 has-[:checked]:border-primary/50 has-[:checked]:bg-primary/8">
            <input type="radio" name="merge-mode" checked={mode === o.value} onChange={() => setMode(o.value)} className="mt-0.5 accent-[var(--primary)]" />
            <span>
              <span className="block text-[13px] font-medium text-foreground">{o.title}</span>
              <span className="block text-[12px] text-muted-foreground">{o.text}</span>
            </span>
          </label>
        ))}
      </div>
      <p className="m-0 text-[12px] text-muted-foreground">If it stops on conflicts you can abort and nothing changes.</p>
    </Modal>
  );
};

export const DeleteBranchDialog: React.FC<{ name: string; merged: boolean; onSubmit: (force: boolean) => void; onClose: () => void }> = ({ name, merged, onSubmit, onClose }) => (
  <Modal
    title={`Delete ${name}?`}
    onClose={onClose}
    footer={
      <>
        <Cancel onClose={onClose} />
        <button type="button" onClick={() => (onSubmit(!merged), onClose())} className="inline-flex h-[30px] items-center rounded-lg bg-destructive px-3 text-[12.5px] font-semibold text-white hover:opacity-90">
          {merged ? "Delete branch" : "Delete anyway"}
        </button>
      </>
    }
  >
    {merged ? (
      <p className="m-0 text-[13px] leading-relaxed text-muted-foreground">Everything on this branch is already in your current branch, so deleting it loses nothing.</p>
    ) : (
      <p className="m-0 text-[13px] leading-relaxed text-warning">
        This branch has commits that are not in your current branch. Deleting it throws those commits away, unless they are pushed somewhere or you note their hash first.
      </p>
    )}
  </Modal>
);

export const TagDialog: React.FC<{ at: string; onSubmit: (name: string, message: string) => void; onClose: () => void }> = ({ at, onSubmit, onClose }) => {
  const [name, setName] = React.useState("");
  const [message, setMessage] = React.useState("");
  const go = () => name.trim() && (onSubmit(name.trim(), message.trim()), onClose());
  return (
    <Modal
      title="New tag"
      onClose={onClose}
      footer={
        <>
          <Cancel onClose={onClose} />
          <PrimaryButton disabled={!name.trim()} onClick={go}>Create tag</PrimaryButton>
        </>
      }
    >
      <Field label="Name">
        <input autoFocus value={name} onChange={(e) => setName(e.target.value)} onKeyDown={onEnter(go)} placeholder="v1.2.0" className={inputClass} />
      </Field>
      <Field label="Message (optional)" hint="With a message it is an annotated tag; without, a plain one.">
        <input value={message} onChange={(e) => setMessage(e.target.value)} onKeyDown={onEnter(go)} className={inputClass} />
      </Field>
      <div className="text-[12.5px] text-muted-foreground">On <span className="font-mono text-foreground">{at}</span>.</div>
    </Modal>
  );
};

export const StashDialog: React.FC<{ onSubmit: (message: string, includeUntracked: boolean) => void; onClose: () => void }> = ({ onSubmit, onClose }) => {
  const [message, setMessage] = React.useState("");
  const [untracked, setUntracked] = React.useState(true);
  return (
    <Modal
      title="Stash changes"
      onClose={onClose}
      footer={
        <>
          <Cancel onClose={onClose} />
          <PrimaryButton onClick={() => (onSubmit(message.trim(), untracked), onClose())}>Stash</PrimaryButton>
        </>
      }
    >
      <p className="m-0 text-[12.5px] text-muted-foreground">Puts your uncommitted work aside and leaves a clean tree. Bring it back from Stashes in the sidebar.</p>
      <Field label="Note (optional)">
        <input autoFocus value={message} onChange={(e) => setMessage(e.target.value)} onKeyDown={onEnter(() => (onSubmit(message.trim(), untracked), onClose()))} placeholder="half-done login form" className={inputClass} />
      </Field>
      <label className="flex items-center gap-2 text-[12.5px] text-foreground">
        <input type="checkbox" checked={untracked} onChange={(e) => setUntracked(e.target.checked)} className="accent-[var(--primary)]" />
        Include new (untracked) files
      </label>
    </Modal>
  );
};

export const ResetDialog: React.FC<{ short: string; subject: string; onSubmit: (mode: ResetMode) => void; onClose: () => void }> = ({ short, subject, onSubmit, onClose }) => {
  const [mode, setMode] = React.useState<ResetMode>("mixed");
  const options: { value: ResetMode; title: string; text: string }[] = [
    { value: "soft", title: "Soft", text: "Moves the branch only. Everything after stays staged, ready to commit again." },
    { value: "mixed", title: "Mixed", text: "Moves the branch and unstages. Your files are untouched." },
    { value: "hard", title: "Hard", text: "Moves the branch and throws away every change after that point, including uncommitted work." },
  ];
  return (
    <Modal
      title={`Reset the current branch to ${short}`}
      onClose={onClose}
      footer={
        <>
          <Cancel onClose={onClose} />
          {mode === "hard" ? (
            <button type="button" onClick={() => (onSubmit(mode), onClose())} className="inline-flex h-[30px] items-center rounded-lg bg-destructive px-3 text-[12.5px] font-semibold text-white hover:opacity-90">
              Reset and discard
            </button>
          ) : (
            <PrimaryButton onClick={() => (onSubmit(mode), onClose())}>Reset</PrimaryButton>
          )}
        </>
      }
    >
      <p className="m-0 truncate text-[12.5px] text-muted-foreground">To: {subject}</p>
      <div className="flex flex-col gap-2">
        {options.map((o) => (
          <label key={o.value} className="flex cursor-pointer gap-3 rounded-lg border border-border p-3 hover:bg-secondary/40 has-[:checked]:border-primary/50 has-[:checked]:bg-primary/8">
            <input type="radio" name="reset-mode" checked={mode === o.value} onChange={() => setMode(o.value)} className="mt-0.5 accent-[var(--primary)]" />
            <span>
              <span className="block text-[13px] font-medium text-foreground">{o.title}</span>
              <span className={`block text-[12px] ${o.value === "hard" ? "text-warning" : "text-muted-foreground"}`}>{o.text}</span>
            </span>
          </label>
        ))}
      </div>
    </Modal>
  );
};

export const PushDialog: React.FC<{
  branch: string;
  hasUpstream: boolean;
  onSubmit: (opts: { setUpstream: boolean; forceWithLease: boolean }) => void;
  onClose: () => void;
}> = ({ branch, hasUpstream, onSubmit, onClose }) => {
  const [force, setForce] = React.useState(false);
  return (
    <Modal
      title={`Push ${branch}`}
      onClose={onClose}
      footer={
        <>
          <Cancel onClose={onClose} />
          <PrimaryButton onClick={() => (onSubmit({ setUpstream: !hasUpstream, forceWithLease: force }), onClose())}>
            {force ? "Force push" : "Push"}
          </PrimaryButton>
        </>
      }
    >
      <p className="m-0 text-[13px] text-muted-foreground">
        {hasUpstream ? "Sends your new commits to the remote branch it tracks." : "This branch is not on the remote yet. Pushing creates it there and links the two."}
      </p>
      {hasUpstream && (
        <label className="flex gap-2 text-[12.5px] text-foreground">
          <input type="checkbox" checked={force} onChange={(e) => setForce(e.target.checked)} className="mt-0.5 accent-[var(--primary)]" />
          <span>
            Force (with lease)
            <span className="block text-[12px] text-warning">Overwrites the remote branch, but only if nobody has pushed to it since you last fetched. Use after a rebase.</span>
          </span>
        </label>
      )}
    </Modal>
  );
};

export const PrDialog: React.FC<{
  repoId: string;
  head: string;
  bases: string[];
  defaultBase: string;
  onClose: () => void;
  onCreated: () => void;
}> = ({ repoId, head, bases, defaultBase, onClose, onCreated }) => {
  const [base, setBase] = React.useState(defaultBase);
  const [title, setTitle] = React.useState("");
  const [body, setBody] = React.useState("");
  const [draft, setDraft] = React.useState(false);
  const [loading, setLoading] = React.useState(true);
  const [creating, setCreating] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [url, setUrl] = React.useState<string | null>(null);

  React.useEffect(() => {
    let live = true;
    setLoading(true);
    bridge
      .gitPrPrefill(repoId, head, base)
      .then((p) => {
        if (!live) return;
        setTitle(p.title);
        setBody(p.body);
      })
      .catch((e) => live && setError(describeBridgeError(e)))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [repoId, head, base]);

  const create = async () => {
    setCreating(true);
    setError(null);
    try {
      const res = await bridge.gitCreatePr(repoId, { head, base, title, body, draft });
      setUrl(res.url);
      onCreated();
    } catch (e) {
      setError(describeBridgeError(e));
    } finally {
      setCreating(false);
    }
  };

  return (
    <Modal
      title={url ? "Pull request created" : `Pull request from ${head}`}
      width={560}
      onClose={onClose}
      footer={
        url ? (
          <>
            <GhostButton onClick={onClose}>Close</GhostButton>
            <PrimaryButton onClick={() => void openUrl(url)}>Open on GitHub</PrimaryButton>
          </>
        ) : (
          <>
            <Cancel onClose={onClose} />
            <PrimaryButton disabled={creating || loading || !title.trim()} onClick={() => void create()}>
              {creating ? "Creating…" : draft ? "Create draft" : "Create pull request"}
            </PrimaryButton>
          </>
        )
      }
    >
      {url ? (
        <p className="m-0 break-all font-mono text-[12.5px] text-info">{url}</p>
      ) : (
        <>
          <p className="m-0 text-[12px] text-muted-foreground">Make sure the branch is pushed first. The pull request is created with the GitHub CLI on the machine the daemon runs on.</p>
          <Field label="Into">
            <select value={base} onChange={(e) => setBase(e.target.value)} className={inputClass}>
              {bases.filter((b) => b !== head).map((b) => (
                <option key={b} value={b}>{b}</option>
              ))}
            </select>
          </Field>
          <Field label="Title">
            <input value={title} onChange={(e) => setTitle(e.target.value)} disabled={loading} className={inputClass} />
          </Field>
          <Field label="Description">
            <textarea value={body} onChange={(e) => setBody(e.target.value)} disabled={loading} className={textareaClass} />
          </Field>
          <label className="flex items-center gap-2 text-[12.5px] text-foreground">
            <input type="checkbox" checked={draft} onChange={(e) => setDraft(e.target.checked)} className="accent-[var(--primary)]" />
            Open as a draft
          </label>
        </>
      )}
      {error && <p className="m-0 whitespace-pre-wrap text-[12.5px] text-destructive">{error}</p>}
    </Modal>
  );
};
