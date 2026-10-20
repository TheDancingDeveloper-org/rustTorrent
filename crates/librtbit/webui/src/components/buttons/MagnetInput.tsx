import { useEffect, useRef, useState } from "react";
import { CgLink } from "react-icons/cg";
import { UploadButton } from "./UploadButton";
import { Modal } from "../modal/Modal";
import { Button } from "./Button";
import { ModalBody } from "../modal/ModalBody";
import { ModalFooter } from "../modal/ModalFooter";
import { FormInput } from "../forms/FormInput";

export const MagnetInput = ({
  className,
  openRequest = 0,
}: {
  className?: string;
  // Bumped by the M shortcut to open the dialog from outside
  openRequest?: number;
}) => {
  const [magnet, setMagnet] = useState<string | null>(null);
  const [inputValue, setInputValue] = useState("");
  const [modalIsOpen, setModalIsOpen] = useState(false);

  // Remounting (leaving the torrents page and coming back) must not replay an
  // earlier shortcut press: only a request newer than the one already handled
  // opens the dialog. Seeded with the mount-time value so a stale counter is
  // consumed silently.
  const handledOpenRequest = useRef(openRequest);
  useEffect(() => {
    if (openRequest === handledOpenRequest.current) return;
    handledOpenRequest.current = openRequest;
    setModalIsOpen(true);
  }, [openRequest]);

  const submit = () => {
    setMagnet(inputValue);
    setInputValue("");
    setModalIsOpen(false);
  };

  const clear = () => {
    setModalIsOpen(false);
    setMagnet(null);
  };

  return (
    <>
      <UploadButton
        onClick={() => {
          setModalIsOpen(true);
        }}
        data={magnet}
        className={`group ${className}`}
        resetData={() => setMagnet(null)}
      >
        <CgLink className="text-primary group-hover:text-on-primary" />
        <div className="whitespace-nowrap">
          <span className="hidden sm:inline">
            Add Torrent from Magnet / URL
          </span>
          <span className="sm:hidden">Magnet</span>
        </div>
      </UploadButton>

      <Modal isOpen={modalIsOpen} onClose={clear} title="Add torrent">
        <ModalBody>
          <FormInput
            autoFocus
            value={inputValue}
            name="magnet"
            onChange={(e) => setInputValue(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !!inputValue) {
                submit();
              }
            }}
            placeholder="magnet:?xt=urn:btih:..."
            help="Enter magnet or HTTP(S) URL to the .torrent"
          />
        </ModalBody>

        <ModalFooter>
          <Button variant="cancel" onClick={clear}>
            Cancel
          </Button>
          <Button disabled={!inputValue} variant="primary" onClick={submit}>
            Add
          </Button>
        </ModalFooter>
      </Modal>
    </>
  );
};
