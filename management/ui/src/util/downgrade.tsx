import { Alert, Box, Button, Text } from "@mantine/core";
import { openModal } from "@mantine/modals";
import { adapter } from "~/adapter";
import { PrimaryTitle } from "~/components/PrimaryTitle";
import { CONFIG_VERSION } from "./defaults";

async function hasConfigBackup() {
    return false;
}

async function restoreBackup() {
    console.warn("Restore backup not supported");
}

export async function showDowngradeWarningModal() {
    const hasBackup = await hasConfigBackup();

    openModal({
        closeOnClickOutside: false,
        closeOnEscape: false,
        title: <PrimaryTitle>Incompatible configuration</PrimaryTitle>,
        children: (
            <Box>
                <Text>
                    Your config file was updated by a newer version of Surrealist and is
                    incompatible with this version.
                </Text>
                {hasBackup ? (
                    <>
                        <Alert
                            mt="xl"
                            color="blue"
                            title="Note"
                        >
                            A backup of your previous configuration file was found. You can restore
                            it by clicking the button below. Note that this will discard any changes
                            you made since the last update.
                        </Alert>
                        <Button
                            style={{ outline: "none" }}
                            onClick={restoreBackup}
                            fullWidth
                            color="blue"
                            mt="md"
                        >
                            Restore backup
                        </Button>
                    </>
                ) : (
                    <Alert
                        mt="xl"
                        color="red"
                        title="Note"
                    >
                        Please reset your configuration file or update your version of Surrealist to
                        continue.
                    </Alert>
                )}
            </Box>
        ),
    });
}
