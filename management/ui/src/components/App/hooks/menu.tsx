import { adapter } from "~/adapter";
import { AppMenu, AppMenuItem } from "~/types";
import { optional } from "~/util/helpers";

const SEPARATOR: AppMenuItem = {
	id: "separator",
	type: "Separator",
};

export function getMenuItems(): AppMenu[] {
	const isDarwin = adapter.platform === "darwin";
	const about: AppMenuItem = {
		id: "open-about",
		name: "About Surrealist",
		type: "Command",
	};

	const settings: AppMenuItem = {
		id: "open-settings",
		name: "Settings",
		type: "Command",
	};

	const surrealistMenu: AppMenu = {
		id: "surrealist",
		name: "Surrealist",
		items: [
			about,
			SEPARATOR,
			settings,
			SEPARATOR,
			{
				id: "hide",
				type: "Hide",
			},
			{
				id: "hide_others",
				type: "HideOthers",
			},
			{
				id: "show_all",
				type: "ShowAll",
			},
			SEPARATOR,
			{
				id: "quit",
				type: "Quit",
				name: "Quit Surrealist",
			},
		],
	};

	const fileMenu: AppMenu = {
		id: "file",
		name: "File",
		items: [
			{
				id: "new-window",
				name: "New Window",
				type: "Command",
			},
			{
				id: "new-connection",
				name: "New Connection",
				type: "Command",
			},
			SEPARATOR,
			{
				id: "open-query-file",
				name: "Open Query File",
				type: "Command",
			},
			SEPARATOR,
			{
				id: "import-database",
				name: "Import Database",
				type: "Command",
			},
			{
				id: "export-database",
				type: "Command",
				name: "Export Database",
			},
			SEPARATOR,
			{
				id: "open-search",
				name: "Open Command Palette",
				type: "Command",
			},
			{
				id: "open-docs",
				name: "Open Documentation Search",
				type: "Command",
			},
			{
				id: "open-connections",
				name: "Open Connection List",
				type: "Command",
			},
			...optional(!isDarwin && [SEPARATOR, settings]),
			SEPARATOR,
			{
				id: "close_window",
				type: "Custom",
				name: "Close Window",
				action: async () => {
					// No-op for web
				},
			},
		],
	};

	const viewMenu: AppMenu = {
		id: "view",
		name: "View",
		items: [
			{
				id: "toggle-win-pinned",
				name: "Toggle Pinned",
				type: "Command",
			},
			SEPARATOR,
			{
				id: "inc-win-scale",
				name: "Zoom In",
				type: "Command",
			},
			{
				id: "dec-win-scale",
				name: "Zoom Out",
				type: "Command",
			},
			SEPARATOR,
			{
				id: "inc-edit-scale",
				name: "Zoom In Editors",
				type: "Command",
			},
			{
				id: "dec-edit-scale",
				name: "Zoom Out Editors",
				type: "Command",
			},
			...optional(isDarwin && SEPARATOR),
		],
	};

	const editMenu: AppMenu = {
		id: "edit",
		name: "Edit",
		items: [
			{
				id: "undo",
				type: "Undo",
			},
			{
				id: "redo",
				type: "Redo",
			},
			SEPARATOR,
			{
				id: "cut",
				type: "Cut",
			},
			{
				id: "copy",
				type: "Copy",
			},
			{
				id: "paste",
				type: "Paste",
			},
			SEPARATOR,
			{
				id: "select-all",
				type: "SelectAll",
			},
		],
	};

	const helpMenu: AppMenu = {
		id: "help",
		name: "Help",
		items: [
			{
				id: "discord",
				type: "Custom",
				name: "Discord",
				action: () => {
					adapter.openUrl("https://discord.gg/surrealdb");
				},
			},
			{
				id: "github",
				type: "Custom",
				name: "GitHub",
				action: () => {
					adapter.openUrl("https://github.com/surrealdb");
				},
			},
			{
				id: "youtube",
				type: "Custom",
				name: "YouTube",
				action: () => {
					adapter.openUrl("https://www.youtube.com/@surrealdb");
				},
			},
			SEPARATOR,
			{
				id: "surrealdb_docs",
				type: "Custom",
				name: "SurrealDB Docs",
				action: () => {
					adapter.openUrl("https://surrealdb.com/docs/surrealdb");
				},
			},
			{
				id: "surrealist_docs",
				type: "Custom",
				name: "Surrealist Docs",
				action: () => {
					adapter.openUrl("https://surrealdb.com/docs/surrealist");
				},
			},
			SEPARATOR,
			{
				id: "fundamentals",
				type: "Custom",
				name: "Fundamentals Course",
				action: () => {
					adapter.openUrl("https://surrealdb.com/learn/fundamentals");
				},
			},
			{
				id: "book",
				type: "Custom",
				name: "Book",
				action: () => {
					adapter.openUrl("https://surrealdb.com/learn/book");
				},
			},
			SEPARATOR,
			{
				id: "report_issue",
				type: "Custom",
				name: "Report Issue",
				action: () => {
					adapter.openUrl("https://github.com/surrealdb/surrealist/issues/new/choose");
				},
			},
			...optional(!isDarwin && about),
		],
	};

	return [
		...optional(isDarwin && surrealistMenu),
		fileMenu,
		...optional(isDarwin && editMenu),
		viewMenu,
		helpMenu,
	];
}

export function useNativeMenuBar() {
	// No-op for web
}
