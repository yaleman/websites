from __future__ import annotations

import datetime
from collections.abc import Mapping
from typing import Any, TypeVar, cast
from uuid import UUID

from attrs import define as _attrs_define
from attrs import field as _attrs_field
from dateutil.parser import isoparse

from ..models.page_type import PageType
from ..types import UNSET, Unset

T = TypeVar("T", bound="ContentItem")


@_attrs_define
class ContentItem:
    """A content item, such as a page or blog post."""

    created_at: datetime.datetime
    creator_sub: str
    draft: bool
    id: UUID
    page_content: str
    page_type: PageType
    site_id: UUID
    slug: str
    title: str
    last_updated: datetime.datetime | None | Unset = UNSET
    published_at: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)

    def to_dict(self) -> dict[str, Any]:
        created_at = self.created_at.isoformat()

        creator_sub = self.creator_sub

        draft = self.draft

        id = str(self.id)

        page_content = self.page_content

        page_type = self.page_type.value

        site_id = str(self.site_id)

        slug = self.slug

        title = self.title

        last_updated: None | str | Unset
        if isinstance(self.last_updated, Unset):
            last_updated = UNSET
        elif isinstance(self.last_updated, datetime.datetime):
            last_updated = self.last_updated.isoformat()
        else:
            last_updated = self.last_updated

        published_at: None | str | Unset
        if isinstance(self.published_at, Unset):
            published_at = UNSET
        elif isinstance(self.published_at, datetime.datetime):
            published_at = self.published_at.isoformat()
        else:
            published_at = self.published_at

        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update(
            {
                "created_at": created_at,
                "creator_sub": creator_sub,
                "draft": draft,
                "id": id,
                "page_content": page_content,
                "page_type": page_type,
                "site_id": site_id,
                "slug": slug,
                "title": title,
            }
        )
        if last_updated is not UNSET:
            field_dict["last_updated"] = last_updated
        if published_at is not UNSET:
            field_dict["published_at"] = published_at

        return field_dict

    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        created_at = isoparse(d.pop("created_at"))

        creator_sub = d.pop("creator_sub")

        draft = d.pop("draft")

        id = UUID(d.pop("id"))

        page_content = d.pop("page_content")

        page_type = PageType(d.pop("page_type"))

        site_id = UUID(d.pop("site_id"))

        slug = d.pop("slug")

        title = d.pop("title")

        def _parse_last_updated(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                last_updated_type_0 = isoparse(data)

                return last_updated_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        last_updated = _parse_last_updated(d.pop("last_updated", UNSET))

        def _parse_published_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                published_at_type_0 = isoparse(data)

                return published_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        published_at = _parse_published_at(d.pop("published_at", UNSET))

        content_item = cls(
            created_at=created_at,
            creator_sub=creator_sub,
            draft=draft,
            id=id,
            page_content=page_content,
            page_type=page_type,
            site_id=site_id,
            slug=slug,
            title=title,
            last_updated=last_updated,
            published_at=published_at,
        )

        content_item.additional_properties = d
        return content_item

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
