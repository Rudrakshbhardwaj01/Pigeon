import { useCallback, useRef, useState } from "react";
import { UploadCloud } from "lucide-react";

interface DropzoneProps {
  accept?: string;
  label: string;
  hint: string;
  onFile: (file: File) => void;
  disabled?: boolean;
  /** Restricts selection to GIFs (receive mode). */
  gifOnly?: boolean;
  /** Called when a file is rejected by the gifOnly filter. */
  onRejected?: (file: File) => void;
}

export function Dropzone({ accept, label, hint, onFile, disabled, gifOnly, onRejected }: DropzoneProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);

  const handleFiles = useCallback(
    (files: FileList | null) => {
      const file = files?.[0];
      if (!file) {
        return;
      }
      if (gifOnly && !file.name.toLowerCase().endsWith(".gif") && file.type !== "image/gif") {
        onRejected?.(file);
        return;
      }
      onFile(file);
    },
    [onFile, gifOnly, onRejected]
  );

  return (
    <div
      className={`dropzone${dragging ? " dragging" : ""}${disabled ? " disabled" : ""}`}
      role="button"
      tabIndex={0}
      aria-label={label}
      aria-disabled={disabled}
      onClick={() => !disabled && inputRef.current?.click()}
      onKeyDown={(e) => {
        if ((e.key === "Enter" || e.key === " ") && !disabled) {
          inputRef.current?.click();
        }
      }}
      onDragOver={(e) => {
        e.preventDefault();
        if (!disabled) setDragging(true);
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={(e) => {
        e.preventDefault();
        setDragging(false);
        if (!disabled) handleFiles(e.dataTransfer.files);
      }}
    >
      <UploadCloud size={30} strokeWidth={1.4} className="dropzone-icon" />
      <div className="dropzone-text">
        <strong>{label}</strong>
        <span>{hint}</span>
      </div>
      <input
        ref={inputRef}
        type="file"
        accept={accept ?? (gifOnly ? "image/gif,.gif" : undefined)}
        hidden
        onChange={(e) => {
          handleFiles(e.target.files);
          e.target.value = "";
        }}
      />
    </div>
  );
}
