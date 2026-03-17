from __future__ import annotations

import datetime
from collections.abc import Mapping
from typing import Any, TypeVar, cast
from uuid import UUID

from attrs import define as _attrs_define
from attrs import field as _attrs_field
from dateutil.parser import isoparse

from ..types import UNSET, Unset

T = TypeVar("T", bound="Asset")


@_attrs_define
class Asset:
    """An uploaded asset, such as an image or file."""

    byte_length: int
    created_at: datetime.datetime
    id: UUID
    mime_type: str
    original_filename: str
    site_id: UUID
    storage_basename: str
    uploader_sub: str
    height: int | None | Unset = UNSET
    width: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)

    def to_dict(self) -> dict[str, Any]:
        byte_length = self.byte_length

        created_at = self.created_at.isoformat()

        id = str(self.id)

        mime_type = self.mime_type

        original_filename = self.original_filename

        site_id = str(self.site_id)

        storage_basename = self.storage_basename

        uploader_sub = self.uploader_sub

        height: int | None | Unset
        if isinstance(self.height, Unset):
            height = UNSET
        else:
            height = self.height

        width: int | None | Unset
        if isinstance(self.width, Unset):
            width = UNSET
        else:
            width = self.width

        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update(
            {
                "byte_length": byte_length,
                "created_at": created_at,
                "id": id,
                "mime_type": mime_type,
                "original_filename": original_filename,
                "site_id": site_id,
                "storage_basename": storage_basename,
                "uploader_sub": uploader_sub,
            }
        )
        if height is not UNSET:
            field_dict["height"] = height
        if width is not UNSET:
            field_dict["width"] = width

        return field_dict

    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        byte_length = d.pop("byte_length")

        created_at = isoparse(d.pop("created_at"))

        id = UUID(d.pop("id"))

        mime_type = d.pop("mime_type")

        original_filename = d.pop("original_filename")

        site_id = UUID(d.pop("site_id"))

        storage_basename = d.pop("storage_basename")

        uploader_sub = d.pop("uploader_sub")

        def _parse_height(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        height = _parse_height(d.pop("height", UNSET))

        def _parse_width(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        width = _parse_width(d.pop("width", UNSET))

        asset = cls(
            byte_length=byte_length,
            created_at=created_at,
            id=id,
            mime_type=mime_type,
            original_filename=original_filename,
            site_id=site_id,
            storage_basename=storage_basename,
            uploader_sub=uploader_sub,
            height=height,
            width=width,
        )

        asset.additional_properties = d
        return asset

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
