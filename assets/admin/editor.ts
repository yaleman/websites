import { Editor, defaultValueCtx, rootCtx } from "@milkdown/core";
import { commonmark } from "@milkdown/preset-commonmark";
import { nord } from "@milkdown/theme-nord";
import { history } from "@milkdown/plugin-history";
import { listener, listenerCtx } from "@milkdown/plugin-listener";
import "./editor.css";

const initEditor = () => {
  const root = document.getElementById("editor");
  const textarea = document.getElementById("page_content") as HTMLTextAreaElement | null;

  if (!root || !textarea) {
    return;
  }

  Editor.make()
    .config(nord)
    .config((editorCtx) => {
      editorCtx.set(rootCtx, root);
      editorCtx.set(defaultValueCtx, textarea.value || "");
      editorCtx.get(listenerCtx).markdownUpdated((_ctx, markdown) => {
        textarea.value = markdown;
      });
    })
    .use(commonmark)
    .use(history)
    .use(listener)
    .create()
    .then(() => {
      textarea.style.display = "none";
      document.body?.classList.add("editor-ready");
    })
    .catch(() => {
      document.body?.classList.add("editor-error");
    });
};

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", initEditor);
} else {
  initEditor();
}
