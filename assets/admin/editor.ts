import { Editor } from "@tiptap/core";
import Image from "@tiptap/extension-image";
import Link from "@tiptap/extension-link";
import { Markdown } from "@tiptap/markdown";
import StarterKit from "@tiptap/starter-kit";
import "./editor.css";
import "./styles.css";

const bindToolbar = (editor: Editor) => {
	const toolbar = document.querySelector<HTMLElement>("[data-editor-toolbar]");
	const previewContainer = document.querySelector<HTMLElement>(
		"[data-editor-preview]",
	);
	const previewBody = document.querySelector<HTMLElement>(
		"[data-editor-preview-body]",
	);
	const previewButton = toolbar?.querySelector<HTMLButtonElement>(
		'button[data-command="preview"]',
	);

	const updatePreview = () => {
		if (!previewBody) {
			return;
		}
		previewBody.innerHTML = editor.getHTML();
	};

	const setPreviewVisible = (visible: boolean) => {
		if (!previewContainer) {
			return;
		}
		if (visible) {
			previewContainer.removeAttribute("hidden");
		} else {
			previewContainer.setAttribute("hidden", "");
		}
		if (previewButton) {
			previewButton.classList.toggle("is-active", visible);
			previewButton.setAttribute("aria-pressed", visible ? "true" : "false");
		}
	};

	const togglePreview = () => {
		if (!previewContainer) {
			return;
		}
		const willShow = previewContainer.hasAttribute("hidden");
		setPreviewVisible(willShow);
		if (willShow) {
			updatePreview();
		}
	};

	if (!toolbar) {
		return { updatePreview };
	}

	const handleCommand = (commandName: string) => {
		switch (commandName) {
			case "bold":
				editor.chain().focus().toggleBold().run();
				return;
			case "italic":
				editor.chain().focus().toggleItalic().run();
				return;
			case "code":
				editor.chain().focus().toggleCode().run();
				return;
			case "link": {
				const href = window.prompt("Link URL");
				if (!href) {
					editor.chain().focus().unsetLink().run();
					return;
				}
				editor.chain().focus().setLink({ href }).run();
				return;
			}
			case "image": {
				const imageUrlInput = document.getElementById(
					"image_url",
				) as HTMLInputElement | null;
				const imageAltInput = document.getElementById(
					"image_alt",
				) as HTMLInputElement | null;
				const src = imageUrlInput?.value.trim() ?? "";
				if (!src) {
					return;
				}
				const alt = imageAltInput?.value.trim();
				const attrs = alt ? { src, alt } : { src };
				editor.chain().focus().setImage(attrs).run();
				if (imageUrlInput) {
					imageUrlInput.value = "";
				}
				if (imageAltInput) {
					imageAltInput.value = "";
				}
				return;
			}
			case "h2":
				editor.chain().focus().toggleHeading({ level: 2 }).run();
				return;
			case "h3":
				editor.chain().focus().toggleHeading({ level: 3 }).run();
				return;
			case "ul":
				editor.chain().focus().toggleBulletList().run();
				return;
			case "ol":
				editor.chain().focus().toggleOrderedList().run();
				return;
			case "quote":
				editor.chain().focus().toggleBlockquote().run();
				return;
			case "preview":
				togglePreview();
				return;
			default:
				return;
		}
	};

	toolbar.addEventListener("click", (event) => {
		const target = event.target as HTMLElement | null;
		if (!target) {
			return;
		}

		const button = target.closest<HTMLButtonElement>("button[data-command]");
		if (!button) {
			return;
		}

		const commandName = button.getAttribute("data-command");
		if (!commandName) {
			return;
		}

		event.preventDefault();
		handleCommand(commandName);
	});

	return { updatePreview };
};

const initEditor = () => {
	const root = document.getElementById("editor");
	const textarea = document.getElementById(
		"page_content",
	) as HTMLTextAreaElement | null;

	if (!root || !textarea) {
		return;
	}

	textarea.style.display = "none";

	let previewControls = { updatePreview: () => {} };

	try {
		const editor = new Editor({
			element: root,
			content: textarea.value || "",
			contentType: "markdown",
      extensions: [
        StarterKit.configure({ link: false }),
        Image,
        Link.configure({ openOnClick: false }),
        Markdown,
      ],
			onUpdate: ({ editor }) => {
				textarea.value = editor.getMarkdown();
				previewControls.updatePreview();
			},
		});

		previewControls = bindToolbar(editor);
		previewControls.updatePreview();
		document.body?.classList.add("editor-ready");
	} catch {
		textarea.style.display = "";
		document.body?.classList.add("editor-error");
	}
};

if (document.readyState === "loading") {
	document.addEventListener("DOMContentLoaded", initEditor);
} else {
	initEditor();
}
