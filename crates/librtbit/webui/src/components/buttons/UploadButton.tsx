import { ReactNode } from "react";
import { useListOnlyUpload } from "../../hooks/useListOnlyUpload";
import { FileSelectionModal } from "../modal/FileSelectionModal";
import { Button } from "./Button";

export const UploadButton: React.FC<{
  onClick: () => void;
  data: string | File | null;
  resetData: () => void;
  children: ReactNode;
  className?: string;
}> = ({ onClick, data, resetData, children, className }) => {
  const { listTorrentResponse, listTorrentLoading, listTorrentError } =
    useListOnlyUpload(data);

  return (
    <>
      <Button onClick={onClick} className={className}>
        {children}
      </Button>

      {data && (
        <FileSelectionModal
          onHide={resetData}
          listTorrentError={listTorrentError}
          listTorrentResponse={listTorrentResponse}
          data={data}
          listTorrentLoading={listTorrentLoading}
        />
      )}
    </>
  );
};
