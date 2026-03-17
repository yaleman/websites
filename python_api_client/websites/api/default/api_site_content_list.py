from http import HTTPStatus
from typing import Any
from urllib.parse import quote
from uuid import UUID

import httpx

from ... import errors
from ...client import AuthenticatedClient, Client
from ...models.api_content_list_response import ApiContentListResponse
from ...models.api_error_response import ApiErrorResponse
from ...types import UNSET, Response, Unset


def _get_kwargs(
    site_id: UUID,
    *,
    page_type: str | Unset = UNSET,
    limit: int | Unset = UNSET,
) -> dict[str, Any]:

    params: dict[str, Any] = {}

    params["page_type"] = page_type

    params["limit"] = limit

    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}

    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/site/{site_id}/content".format(
            site_id=quote(str(site_id), safe=""),
        ),
        "params": params,
    }

    return _kwargs


def _parse_response(
    *, client: AuthenticatedClient | Client, response: httpx.Response
) -> ApiContentListResponse | ApiErrorResponse | None:
    if response.status_code == 200:
        response_200 = ApiContentListResponse.from_dict(response.json())

        return response_200

    if response.status_code == 400:
        response_400 = ApiErrorResponse.from_dict(response.json())

        return response_400

    if response.status_code == 401:
        response_401 = ApiErrorResponse.from_dict(response.json())

        return response_401

    if response.status_code == 403:
        response_403 = ApiErrorResponse.from_dict(response.json())

        return response_403

    if response.status_code == 500:
        response_500 = ApiErrorResponse.from_dict(response.json())

        return response_500

    if client.raise_on_unexpected_status:
        raise errors.UnexpectedStatus(response.status_code, response.content)
    else:
        return None


def _build_response(
    *, client: AuthenticatedClient | Client, response: httpx.Response
) -> Response[ApiContentListResponse | ApiErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    site_id: UUID,
    *,
    client: AuthenticatedClient,
    page_type: str | Unset = UNSET,
    limit: int | Unset = UNSET,
) -> Response[ApiContentListResponse | ApiErrorResponse]:
    """
    Args:
        site_id (UUID):
        page_type (str | Unset):
        limit (int | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ApiContentListResponse | ApiErrorResponse]
    """

    kwargs = _get_kwargs(
        site_id=site_id,
        page_type=page_type,
        limit=limit,
    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)


def sync(
    site_id: UUID,
    *,
    client: AuthenticatedClient,
    page_type: str | Unset = UNSET,
    limit: int | Unset = UNSET,
) -> ApiContentListResponse | ApiErrorResponse | None:
    """
    Args:
        site_id (UUID):
        page_type (str | Unset):
        limit (int | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ApiContentListResponse | ApiErrorResponse
    """

    return sync_detailed(
        site_id=site_id,
        client=client,
        page_type=page_type,
        limit=limit,
    ).parsed


async def asyncio_detailed(
    site_id: UUID,
    *,
    client: AuthenticatedClient,
    page_type: str | Unset = UNSET,
    limit: int | Unset = UNSET,
) -> Response[ApiContentListResponse | ApiErrorResponse]:
    """
    Args:
        site_id (UUID):
        page_type (str | Unset):
        limit (int | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ApiContentListResponse | ApiErrorResponse]
    """

    kwargs = _get_kwargs(
        site_id=site_id,
        page_type=page_type,
        limit=limit,
    )

    response = await client.get_async_httpx_client().request(**kwargs)

    return _build_response(client=client, response=response)


async def asyncio(
    site_id: UUID,
    *,
    client: AuthenticatedClient,
    page_type: str | Unset = UNSET,
    limit: int | Unset = UNSET,
) -> ApiContentListResponse | ApiErrorResponse | None:
    """
    Args:
        site_id (UUID):
        page_type (str | Unset):
        limit (int | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ApiContentListResponse | ApiErrorResponse
    """

    return (
        await asyncio_detailed(
            site_id=site_id,
            client=client,
            page_type=page_type,
            limit=limit,
        )
    ).parsed
