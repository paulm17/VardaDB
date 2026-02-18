import { Box, Group, Image, Menu, Text } from "@mantine/core";
import { useState } from "react";
import icon from "~/assets/images/icon.webp";
import { Icon } from "~/components/Icon";
import {
	Command,
	useCommandCategories,
	useCommandDispatcher,
	useCommandKeybinds,
} from "~/providers/Commands";
import { displayBinding } from "~/providers/Commands/keybindings";
import { useInterfaceStore } from "~/stores/interface";
import { ActionButton } from "../ActionButton";
import { getMenuItems } from "../App/hooks/menu";
import { Spacer } from "../Spacer";
import classes from "./style.module.scss";

export function AppTitleBar() {
	const keybinds = useCommandKeybinds();
	const cmdCategories = useCommandCategories();
	const dispatchCommand = useCommandDispatcher();
	const menuItems = getMenuItems();

	const { title } = useInterfaceStore.getState();

	const commands = cmdCategories.reduce((acc, category) => {
		for (const command of category.commands) {
			acc.set(command.id, command);
		}
		return acc;
	}, new Map<string, Command>());

	return (
		<Box className={classes.titleBar}>
			<Group gap={0}>
				<Image
					src={icon}
					w={23}
					m="md"
				/>
				{menuItems
					?.filter((it) => !it.disabled)
					.map((menu) => (
						<Menu
							key={menu.id}
							position="bottom-start"
						>
							<Menu.Target key={`${menu.id}-target`}>
								<Text
									size="md"
									className={classes.menuButton}
								>
									{menu.name}
								</Text>
							</Menu.Target>

							<Menu.Dropdown
								key={`${menu.id}-dropdown`}
								p="xs"
							>
								{menu.items.map((item, index) => {
									if (item.type === "Separator") {
										return <Menu.Divider key={index} />;
									}

									if (item.type === "Command") {
										const command = commands.get(item.id);
										const disabled = item.disabled || !command;

										return (
											<Menu.Item
												key={item.id}
												disabled={disabled}
												onClick={() => {
													dispatchCommand(item.id, item.data);
												}}
											>
												<Group gap={8}>
													{item.name}
													{keybinds.has(item.id) && (
														<>
															<Spacer />
															<Text c="slate.4">
																<Group
																	gap={2}
																	wrap="nowrap"
																>
																	{displayBinding(
																		keybinds.get(item.id) ?? [],
																	)}
																</Group>
															</Text>
														</>
													)}
												</Group>
											</Menu.Item>
										);
									}

									if (item.type === "Custom") {
										return (
											<Menu.Item
												key={item.id}
												disabled={item.disabled}
												onClick={item.action}
											>
												{item.name}
											</Menu.Item>
										);
									}
								})}
							</Menu.Dropdown>
						</Menu>
					))}
			</Group>

			<Box
				className={classes.dragArea}
			>
				<Text
					c="bright"
					fz="lg"
					fw={500}
				>
					{title}
				</Text>
			</Box>

			{/* Window controls removed for web */}
			<Group
				gap={0}
				className={classes.windowControls}
			>
			</Group>
		</Box>
	);
}
