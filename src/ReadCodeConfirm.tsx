import { Button, Dialog, Portal } from "@chakra-ui/react";
import { useRef } from "react";

export type ReadCodeConfirmProps = {
  isOpen: boolean;
  onClose: () => void;
  onConfirm: () => void;
};

/** Dialog for the "read the code" button when it clears the editor. */
function ReadCodeConfirm({ isOpen, onClose, onConfirm }: ReadCodeConfirmProps) {
  const cancelRef = useRef<HTMLButtonElement>(null);

  return (
    <Dialog.Root
      role="alertdialog"
      open={isOpen}
      onOpenChange={(details) => !details.open && onClose()}
      initialFocusEl={() => cancelRef.current}
      placement="center"
    >
      <Portal>
        <Dialog.Backdrop />
        <Dialog.Positioner>
          <Dialog.Content>
            <Dialog.Header>
              <Dialog.Title>Clear editor</Dialog.Title>
            </Dialog.Header>

            <Dialog.Body>
              Opening Rustpad's source code will clear the existing shared
              content. Is this okay?
            </Dialog.Body>

            <Dialog.Footer>
              <Button ref={cancelRef} variant="outline" onClick={onClose}>
                Cancel
              </Button>
              <Button colorPalette="red" onClick={onConfirm}>
                Clear
              </Button>
            </Dialog.Footer>
          </Dialog.Content>
        </Dialog.Positioner>
      </Portal>
    </Dialog.Root>
  );
}

export default ReadCodeConfirm;
